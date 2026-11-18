//! init 主循环 (pm-sysd-init 的常驻逻辑).
//!
//! 承接 cli::init 解析后的实例与配置路径, 完成 IPC 监听, 配置加载,
//! service 启动, 连接处理. 退出 与 soft-reboot 由 SysD 决定, 通过 `exit_tx`
//! 通知本模块.

use std::{future, sync::Arc};

use log::{debug, error, trace, warn};
use serde::Serialize;
use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::{
    jsonl::{
        Hello, SdReq, SdReqSysd, SdRes, check_version, decode_frame, decode_hello, encode_frame,
        encode_hello,
    },
    logd::Logd,
    pa::{Instance, IpcAcl, PaError, SysdServer, SysdServerConn, spawn_signal_watch},
    sysu::reached_target,
};

use super::{SysD, SysDM, SysdError, TARGET_DEFAULT};

#[cfg(test)]
mod test;

use super::reload::reload;

/// 最大并发连接数 (同时最多接受的 ctl/log 连接).
const MAX_CONNECTIONS: usize = 200;

/// 退出方式.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Shutdown {
    /// 正常退出.
    Exit,
    /// 重启自身 (soft-reboot).
    SoftReboot,
}

/// init 主流程.
///
/// `print_version` 由 bin 传入 (pm-bin 生成的版本输出函数),
/// 在完成实例唯一性检测后调用, 向控制台输出版本信息.
pub async fn run(
    instance: Instance,
    conf_path: String,
    print_version: fn(),
) -> Result<Shutdown, SysdError> {
    trace!("start");

    // (1) 首先进行实例唯一性检测 (早于其余所有操作).
    // 成功即持有 IPC 监听 (UNIX socket / 命名管道), 防止 TOCTOU.
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
    // Windows: --system 实例另外创建只读 (_ro) 命名管道.
    // 创建 ro 管道也是实例唯一性检测的一部分, 失败同样退出.
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
    // 实例唯一性检测已完成, 继续启动前输出版本信息
    print_version();

    debug!(
        "pm-sysd-init `{}` listening on `{}`",
        instance.name(),
        transport
    );

    // (2) 加载配置与 mask/enable 状态 (首次调用, 数据库由加载线程打开并返回). 失败直接退出.
    let (loaded, db) = reload(None, &conf_path).await?;
    let vacuum_db = db.clone();

    // (3) 其余后续处理
    let logd = Logd::new(db.clone());

    // 退出信号: SysD 决定退出时 通过 exit_tx 通知主循环
    let (exit_tx, mut exit_rx) = oneshot::channel::<Shutdown>();
    for w in &loaded.conf.warnings {
        warn!("{}", w);
        logd.write_with("warn", None, w).expect("write warning log");
    }
    let sysd = SysD::spawn(db, logd.clone(), &conf_path, loaded, exit_tx);

    let (tx, rx) = oneshot::channel();
    if sysd.send(SysDM::StartDefaultTarget(tx)).await.is_err() {
        error!("send start-default-target failed (sysd stopped)");
    } else {
        match rx.await {
            Ok(Ok(())) => {
                // 模仿 systemd: 到达 default.target 时输出成功行
                println!("{}", reached_target(TARGET_DEFAULT));
            }
            Ok(Err(e)) => error!("start default target: {}", e),
            Err(_) => error!("start default target: sysd stopped before reply"),
        }
    }
    logd.write_with("info", None, "pm-sysd-init started")
        .expect("write startup log");

    // 并发连接限制.
    let sem = Arc::new(Semaphore::new(MAX_CONNECTIONS));

    // 外部信号 (SIGTERM / Ctrl+C).
    let (sig_tx, mut sig_rx) = mpsc::channel::<()>(1);
    spawn_signal_watch(sig_tx);

    let shutdown;
    let mut conn_seq: u64 = 0;
    loop {
        tokio::select! {
            r = &mut exit_rx => {
                shutdown = r.unwrap_or(Shutdown::Exit);
                break;
            }
            _ = sig_rx.recv() => {
                // 外部信号: 交给 SysD 统一处理 (发送 exit 命令)
                let (tx, rx) = oneshot::channel();
                if sysd
                    .send(SysDM::SdReq(SdReq::SysD(SdReqSysd::Exit), tx))
                    .await
                    .is_err()
                {
                    error!("send exit signal to sysd failed (sysd stopped)");
                    continue;
                }
                if rx.await.is_err() {
                    error!("exit signal: sysd stopped before reply");
                }
            }
            accepted = listener.accept() => {
                handle_accept(accepted, false, sysd.clone(), &sem, &mut conn_seq);
            }
            accepted_ro = ro_accept(&mut ro_listener) => {
                handle_accept(accepted_ro, true, sysd.clone(), &sem, &mut conn_seq);
            }
        }
    }

    // 优雅退出: 记录日志, 等待日志落库 (service 已由 SysD 通过 Exit 停止).
    logd.write_with("info", None, "pm-sysd-init exiting")
        .expect("write exit log");
    if let Err(e) = logd.flush().await {
        error!("flush logs: {}", e);
    }
    // 退出前 (soft-reboot 重启自身前) 对数据库执行 vacuum.
    if let Err(e) = vacuum_db.vacuum().await {
        error!("vacuum database: {}", e);
    }
    trace!("exit {:?}", shutdown);
    Ok(shutdown)
}

/// 处理一个被接受的连接 (应用并发限制后交给 [`handle_conn`]).
///
/// `conn_seq` 为连接序号计数器, 每次接受连接时递增 (从 1 开始).
fn handle_accept(
    accepted: Result<SysdServerConn, PaError>,
    ro: bool,
    sysd: SysD,
    sem: &Arc<Semaphore>,
    conn_seq: &mut u64,
) {
    match accepted {
        Ok(conn) => {
            *conn_seq += 1;
            let seq = *conn_seq;
            trace!("accept #{} ro={}", seq, ro);
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
                handle_conn(conn, sysd, ro, seq).await;
            });
        }
        Err(e) => {
            error!("accept: {}", e);
        }
    }
}

/// 连接关闭时的调试跟踪 guard (在 `handle_conn` 返回时触发)
struct ConnGuard {
    seq: u64,
    ro: bool,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        trace!("conn #{} close ro={}", self.seq, self.ro);
    }
}

/// 处理单个 IPC 连接.
async fn handle_conn(conn: SysdServerConn, sysd: SysD, ro: bool, seq: u64) {
    let _close = ConnGuard { seq, ro };
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
            if let Err(e) = conn.send(line).await {
                error!("send handshake version: {}", e);
            }
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
        if ro && !cmd.is_readonly() {
            error!("read-only connection attempted a modifying command");
            if let Ok(line) = encode_frame(&SdRes::Err("read-only connection".to_string())) {
                if let Err(e) = conn.send(line).await {
                    error!("send read-only error response: {}", e);
                }
            }
            return;
        }

        let (tx, rx) = oneshot::channel();
        if sysd.send(SysDM::SdReq(cmd, tx)).await.is_err() {
            error!("send command to sysd failed (sysd stopped)");
            if let Ok(line) = encode_frame(&SdRes::Err("sysd stopped".to_string())) {
                if let Err(e) = conn.send(line).await {
                    error!("send sysd-stopped error response: {}", e);
                }
            }
            return;
        }

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

/// 等待只读监听器的下一个连接; 无只读监听器时永远 pending.
async fn ro_accept(l: &mut Option<SysdServer>) -> Result<SysdServerConn, PaError> {
    match l.as_mut() {
        Some(l) => l.accept().await,
        None => future::pending::<Result<SysdServerConn, PaError>>().await,
    }
}
