//! init 主循环 (pm-sysd-init 的常驻逻辑).
//!
//! 承接 cli::init 解析后的实例与配置路径, 完成 IPC 监听, 配置加载,
//! service 启动, 连接处理. 退出 与 soft-reboot 由 SysD 决定, 通过 `exit_tx`
//! 通知本模块.

use std::{future, sync::Arc};

use log::{error, info, warn};
use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::{
    conf::load_conf,
    db::DbClient,
    jsonl::{
        Hello, SdReq, SdReqSysd, SdRes, check_version, decode_frame, decode_hello, encode_frame,
        encode_hello,
    },
    logd::Logd,
    pa::{Instance, IpcAcl, PaError, SysdServer, SysdServerConn, spawn_signal_watch},
};

use super::{SysD, SysDM, SysdError};

/// 最大并发连接数 (同时最多接受的 ctl/log 连接).
const MAX_CONNECTIONS: usize = 200;

/// 退出方式.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shutdown {
    /// 正常退出.
    Exit,
    /// 重启自身 (soft-reboot).
    SoftReboot,
}

/// init 主流程.
pub async fn run(instance: Instance, conf_path: String) -> Result<Shutdown, SysdError> {
    // (1) 首先进行 实例唯一性检测 (早于其余所有操作). 成功即持有 IPC 监听
    // (UNIX socket / 命名管道), 防止 TOCTOU.
    let transport = instance.transport();
    let acl = if instance == Instance::System {
        IpcAcl::System
    } else {
        IpcAcl::CurrentUser
    };
    let mut listener = SysdServer::bind(&transport, acl).map_err(|e| {
        SysdError::Msg(format!(
            "instance `{}` ({}): {}",
            instance.name(),
            transport,
            e
        ))
    })?;

    // Windows: --system 实例另外创建 只读 (_ro) 命名管道. 创建 ro 管道也是
    // 实例唯一性检测 的一部分, 失败同样退出.
    let mut ro_listener = match instance.ro_transport() {
        Some(ro_transport) => Some(
            SysdServer::bind(&ro_transport, IpcAcl::SystemReadOnly).map_err(|e| {
                SysdError::Msg(format!(
                    "instance `{}` ({}): {}",
                    instance.name(),
                    ro_transport,
                    e
                ))
            })?,
        ),
        None => None,
    };

    info!(
        "pm-sysd-init `{}` listening on `{}`",
        instance.name(),
        transport
    );

    // (2) 加载配置文件. 失败直接退出.
    let conf = load_conf(&conf_path).map_err(|e| SysdError::Msg(e.to_string()))?;

    // (3) 打开数据库.
    let db = DbClient::open(&conf.db_path).map_err(|e| SysdError::Msg(e.to_string()))?;

    // (4) 其余后续处理.
    let logd = Logd::new(db.clone());

    // 退出信号: SysD 决定退出时 通过 exit_tx 通知主循环.
    let (exit_tx, mut exit_rx) = mpsc::channel::<Shutdown>(1);
    let sysd = SysD::spawn(db, logd.clone(), &conf_path, conf.units, exit_tx);
    for w in &conf.warnings {
        warn!("{}", w);
        logd.write_with("warn", None, w).expect("write warning log");
    }

    let (tx, rx) = oneshot::channel();
    sysd.send(SysDM::StartDefaultTarget(tx)).await.unwrap();
    tokio::spawn(async move {
        if let Ok(Err(e)) = rx.await {
            error!("start default target: {}", e);
        }
    });
    logd.write_with("info", None, "pm-sysd-init started")
        .expect("write startup log");

    // 并发连接限制.
    let sem = Arc::new(Semaphore::new(MAX_CONNECTIONS));

    // 外部信号 (SIGTERM / Ctrl+C).
    let (sig_tx, mut sig_rx) = mpsc::channel::<()>(1);
    spawn_signal_watch(sig_tx);

    let shutdown;
    loop {
        tokio::select! {
            r = exit_rx.recv() => {
                shutdown = r.unwrap_or(Shutdown::Exit);
                break;
            }
            _ = sig_rx.recv() => {
                // 外部信号: 交给 SysD 统一处理 (发送 exit 命令)
                let (tx, rx) = oneshot::channel();
                sysd.send(SysDM::SdReq((SdReq::SysD(SdReqSysd::Exit), tx)))
                    .await
                    .unwrap();
                rx.await.unwrap();
            }
            accepted = listener.accept() => {
                handle_accept(accepted, false, sysd.clone(), &sem);
            }
            accepted_ro = ro_accept(&mut ro_listener) => {
                handle_accept(accepted_ro, true, sysd.clone(), &sem);
            }
        }
    }

    // 优雅退出: 记录日志, 等待日志落库 (service 已由 SysD 通过 Exit 停止).
    logd.write_with("info", None, "pm-sysd-init exiting")
        .expect("write exit log");
    logd.flush().expect("flush logs");
    Ok(shutdown)
}

/// 处理一个被接受的连接 (应用并发限制后交给 [`handle_conn`]).
fn handle_accept(
    accepted: Result<SysdServerConn, PaError>,
    ro: bool,
    sysd: SysD,
    sem: &Arc<Semaphore>,
) {
    match accepted {
        Ok(conn) => {
            let permit = match sem.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    error!("too many concurrent connections (max {})", MAX_CONNECTIONS);
                    drop(conn);
                    return;
                }
            };
            tokio::spawn(async move {
                let _permit = permit;
                handle_conn(conn, sysd, ro).await;
            });
        }
        Err(e) => {
            error!("accept: {}", e);
        }
    }
}

/// 处理单个 IPC 连接.
async fn handle_conn(conn: SysdServerConn, sysd: SysD, ro: bool) {
    let mut conn = conn;

    // 握手: 无法解析则报错并关闭连接.
    let hello = match conn.recv().await {
        Ok(line) => match decode_hello(&line) {
            Ok(h) => h,
            Err(e) => {
                error!("handshake decode: {}", e);
                return;
            }
        },
        Err(PaError::Eof) => return,
        Err(e) => {
            error!("handshake recv: {}", e);
            return;
        }
    };

    // 版本不一致: 向对方发送自己的版本, 然后关闭连接.
    if let Err(e) = check_version(&hello) {
        error!("handshake version: {}", e);
        if let Ok(line) = encode_hello(&Hello::new()) {
            conn.send(line).await.expect("send handshake version");
        }
        return;
    }

    match encode_hello(&Hello::new()) {
        Ok(line) => {
            if let Err(e) = conn.send(line).await {
                error!("handshake send: {}", e);
                return;
            }
        }
        Err(e) => {
            error!("handshake encode: {}", e);
            return;
        }
    }

    loop {
        let cmd = match conn.recv().await {
            Ok(line) => match decode_frame::<SdReq>(&line) {
                Ok(c) => c,
                Err(e) => {
                    error!("decode command: {}", e);
                    return;
                }
            },
            Err(PaError::Eof) => {
                // 客户端正常关闭连接.
                return;
            }
            Err(e) => {
                error!("recv: {}", e);
                return;
            }
        };

        // 只读连接只能只读查询, 不能修改.
        if ro && !is_readonly(&cmd) {
            error!("read-only connection attempted a modifying command");
            if let Ok(line) = encode_frame(&SdRes::Err("read-only connection".to_string())) {
                conn.send(line)
                    .await
                    .expect("send read-only error response");
            }
            return;
        }

        let (tx, rx) = oneshot::channel();
        sysd.send(SysDM::SdReq((cmd, tx))).await.unwrap();

        let resp = rx
            .await
            .unwrap_or_else(|_| SdRes::Err("sysd stopped".to_string()));
        match encode_frame(&resp) {
            Ok(line) => {
                if let Err(e) = conn.send(line).await {
                    error!("send response: {}", e);
                    return;
                }
            }
            Err(e) => {
                error!("encode response: {}", e);
                return;
            }
        }
    }
}

/// 判断命令是否 只读 (允许从只读连接发出)
fn is_readonly(cmd: &SdReq) -> bool {
    matches!(cmd, SdReq::LogRO(_) | SdReq::UnitdRO(_) | SdReq::UnitRO(_))
}

/// 等待只读监听器的下一个连接; 无只读监听器时永远 pending.
async fn ro_accept(l: &mut Option<SysdServer>) -> Result<SysdServerConn, PaError> {
    match l.as_mut() {
        Some(l) => l.accept().await,
        None => future::pending::<Result<SysdServerConn, PaError>>().await,
    }
}
