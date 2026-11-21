//! Server: IPC 监听 / 连接处理 / 实例唯一性检测 / 退出协调.
//!
//! [`Server::new`] 创建 [`Server`] 与 [`ServerSender`]. 其它任务 (init 启动任务,
//! SysD actor) 通过 [`ServerSender`] 给 Server 主循环发消息.
//!
//! Server 主循环只负责: 接受连接并转发给 SysD, 监听外部信号, 汇聚退出结果.
//! 配置加载 / SysD 创建等初始化工作由 init 启动任务完成.

use std::sync::Arc;

use log::{debug, error, trace};
use serde::Serialize;
use tokio::sync::{Semaphore, mpsc, oneshot, watch};

use crate::{
    conf::limit::SysdLimit,
    jsonl::{
        Hello, SdReq, SdReqSysd, SdReqUnitdRO, SdRes, check_version, decode_frame, decode_hello,
        encode_frame, encode_hello,
    },
    pa::{Instance, IpcAcl, PaError, SysdConn, SysdListener, spawn_signal_watch},
    sysu::DaemonSta,
};

use super::{SysD, SysDM, SysdError};

#[cfg(test)]
mod test;

/// 退出方式.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Shutdown {
    /// 正常退出.
    Exit,
    /// 重启自身 (soft-reboot).
    SoftReboot,
}

/// 发给 Server 主循环的控制消息.
enum ServerMsg {
    /// 设置 SysD 句柄 (启动完成后, 由 init 启动任务发送).
    SetSysd(SysD),
    /// 更新限制配置 (配置加载后发送).
    SetLimit(SysdLimit),
    /// 标记开始退出 (由 SysD 发送), 之后 `is-system-running` 直接返回 stopping.
    MarkExit,
    /// 请求退出, 携带退出方式或错误 (由 SysD / init 启动任务发送).
    Exit(Result<Shutdown, SysdError>),
}

/// Server 主循环.
pub struct Server {
    /// 实例 (决定 IPC 地址与 ACL).
    instance: Instance,
    /// 当前限制配置.
    limit: SysdLimit,
    /// 主管道 / UNIX socket 监听器 (`bind` 后为 Some).
    listener: Option<SysdListener>,
    /// 只读管道监听器 (仅 Windows 的 `--system` 实例).
    ro_listener: Option<SysdListener>,
    /// 控制消息接收端.
    rx: mpsc::Receiver<ServerMsg>,
    /// SysD 句柄广播 (连接处理 task 读取当前值).
    sysd_tx: watch::Sender<Option<SysD>>,
    /// SysD 句柄广播接收端 (始终保留一个, 保证发送端可写).
    sysd_rx: watch::Receiver<Option<SysD>>,
    /// 退出标记广播.
    exiting_tx: watch::Sender<bool>,
    /// 退出标记广播接收端 (始终保留一个, 保证发送端可写).
    exiting_rx: watch::Receiver<bool>,
}

impl Server {
    /// 创建 Server 与对应的 [`ServerSender`].
    pub fn new(instance: Instance, limit: SysdLimit) -> (Self, ServerSender) {
        let (msg_tx, msg_rx) = mpsc::channel(limit.channel_capacity_n);
        let (sysd_tx, sysd_rx) = watch::channel::<Option<SysD>>(None);
        let (exiting_tx, exiting_rx) = watch::channel::<bool>(false);

        let sender = ServerSender { tx: msg_tx };
        let server = Self {
            instance,
            limit,
            listener: None,
            ro_listener: None,
            rx: msg_rx,
            sysd_tx,
            sysd_rx,
            exiting_tx,
            exiting_rx,
        };
        (server, sender)
    }

    /// 绑定 IPC 监听 (同时完成实例唯一性检测).
    ///
    /// 成功后即持有监听器, 防止 TOCTOU.
    pub async fn bind(&mut self) -> Result<(), SysdError> {
        let transport = self.instance.transport();
        let acl = if self.instance == Instance::System {
            IpcAcl::System
        } else {
            IpcAcl::CurrentUser
        };
        self.listener = Some(
            SysdListener::bind(&transport, acl, self.limit.ipc()).map_err(|e| {
                SysdError::Msg(format!(
                    "instance `{}` ({}): {}",
                    self.instance.name(),
                    transport,
                    e
                ))
            })?,
        );
        // Windows: --system 实例另外创建只读 (_ro) 命名管道.
        // 创建 ro 管道也是实例唯一性检测的一部分, 失败同样退出.
        self.ro_listener = match self.instance.ro_transport() {
            Some(ro_transport) => Some(
                SysdListener::bind(&ro_transport, IpcAcl::SystemReadOnly, self.limit.ipc())
                    .map_err(|e| {
                        SysdError::Msg(format!(
                            "instance `{}` ({}): {}",
                            self.instance.name(),
                            ro_transport,
                            e
                        ))
                    })?,
            ),
            None => None,
        };

        debug!(
            "pm-sysd-init `{}` listening on `{}`",
            self.instance.name(),
            transport
        );
        Ok(())
    }

    /// 进入接受连接的主循环, 直到退出/错误后才返回.
    ///
    /// 返回的 [`Shutdown`] 由 SysD (或启动失败路径) 通过 [`ServerSender::exit`]
    /// 传入.
    pub async fn loop_accept(self) -> Result<Shutdown, SysdError> {
        let Self {
            listener,
            ro_listener,
            mut rx,
            sysd_tx,
            sysd_rx,
            exiting_tx,
            exiting_rx,
            mut limit,
            ..
        } = self;

        // 并发连接限制: 主管道与只读管道分开计算.
        let mut main_sem = Arc::new(Semaphore::new(limit.ipc_conn_n));
        let mut ro_sem = Arc::new(Semaphore::new(limit.ipc_conn_ro_n));

        // 外部信号 (SIGTERM / Ctrl+C).
        let (sig_tx, mut sig_rx) = mpsc::channel::<()>(1);
        spawn_signal_watch(sig_tx);

        // 接受连接放在独立任务中: 若直接放在 select 分支里, 分支未被选中时
        // 已建立的连接会被取消并丢弃 (Windows 上表现为客户端写失败
        // "The pipe is being closed"). 独立任务把已接受的连接送入通道,
        // 主循环取消通道接收不会丢失连接.
        let (main_tx, mut main_rx) =
            mpsc::channel::<Result<SysdConn, PaError>>(limit.channel_capacity_n);
        let (ro_tx, mut ro_rx) =
            mpsc::channel::<Result<SysdConn, PaError>>(limit.channel_capacity_n);
        let main_task = match listener {
            Some(l) => Some(tokio::spawn(accept_loop(l, main_tx))),
            None => None,
        };
        let ro_task = match ro_listener {
            Some(l) => Some(tokio::spawn(accept_loop(l, ro_tx))),
            None => None,
        };

        let mut conn_seq: u64 = 0;
        trace!("accept loop start");

        let result = loop {
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(ServerMsg::SetSysd(sysd)) => {
                            if sysd_tx.send(Some(sysd)).is_err() {
                                error!("send sysd handle to connection tasks failed");
                            }
                        }
                        Some(ServerMsg::SetLimit(l)) => {
                            limit = l;
                            main_sem = Arc::new(Semaphore::new(limit.ipc_conn_n));
                            ro_sem = Arc::new(Semaphore::new(limit.ipc_conn_ro_n));
                        }
                        Some(ServerMsg::MarkExit) => {
                            if exiting_tx.send(true).is_err() {
                                error!("send exiting flag to connection tasks failed");
                            }
                        }
                        Some(ServerMsg::Exit(r)) => {
                            trace!("accept loop exit: {:?}", r);
                            break r;
                        }
                        None => {
                            error!("server sender dropped before exit");
                            break Err(SysdError::Failed("server sender dropped".to_string()));
                        }
                    }
                }
                _ = sig_rx.recv() => {
                    handle_signal(&sysd_rx).await;
                }
                accepted = main_rx.recv() => {
                    match accepted {
                        Some(accepted) => handle_accept(
                            accepted,
                            false,
                            Conn::new(sysd_rx.clone(), exiting_rx.clone()),
                            &main_sem,
                            &mut conn_seq,
                            &limit,
                        ),
                        None => {
                            error!("main accept task stopped unexpectedly");
                            break Err(SysdError::Failed("main accept task stopped".to_string()));
                        }
                    }
                }
                accepted_ro = ro_rx.recv() => {
                    match accepted_ro {
                        Some(accepted_ro) => handle_accept(
                            accepted_ro,
                            true,
                            Conn::new(sysd_rx.clone(), exiting_rx.clone()),
                            &ro_sem,
                            &mut conn_seq,
                            &limit,
                        ),
                        None => {
                            error!("ro accept task stopped unexpectedly");
                            break Err(SysdError::Failed("ro accept task stopped".to_string()));
                        }
                    }
                }
            }
        };

        // 结束接受任务并等待退出, 确保 listener 被 Drop (Linux 删除 socket 文件).
        if let Some(task) = main_task {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = ro_task {
            task.abort();
            let _ = task.await;
        }

        result
    }
}

/// Server 控制句柄, 可以从多个任务克隆并发送消息.
#[derive(Clone)]
pub struct ServerSender {
    tx: mpsc::Sender<ServerMsg>,
}

impl ServerSender {
    /// 请求退出 (等待消息投递完成, 不等待处理完成).
    ///
    /// `r` 为退出方式或错误. 错误会从 [`Server::loop_accept`] 返回.
    pub async fn exit(&self, r: Result<Shutdown, SysdError>) -> Result<(), SysdError> {
        self.tx
            .send(ServerMsg::Exit(r))
            .await
            .map_err(|_| SysdError::Failed("server stopped".to_string()))
    }

    /// 标记开始退出 (由 SysD 在开始退出序列时发送).
    ///
    /// 之后 Server 对 `is-system-running` 请求直接拦截并返回 stopping.
    pub async fn mark_exit(&self) -> Result<(), SysdError> {
        self.tx
            .send(ServerMsg::MarkExit)
            .await
            .map_err(|_| SysdError::Failed("server stopped".to_string()))
    }

    /// 设置 SysD 句柄 (由 init 启动任务在 SysD 创建后发送).
    pub async fn set_sysd(&self, sysd: SysD) -> Result<(), SysdError> {
        self.tx
            .send(ServerMsg::SetSysd(sysd))
            .await
            .map_err(|_| SysdError::Failed("server stopped".to_string()))
    }

    /// 更新限制配置 (由 init 启动任务在配置加载后发送).
    pub async fn set_limit(&self, limit: SysdLimit) -> Result<(), SysdError> {
        self.tx
            .send(ServerMsg::SetLimit(limit))
            .await
            .map_err(|_| SysdError::Failed("server stopped".to_string()))
    }
}

/// 连接处理 task 使用的共享状态 (SysD 句柄与退出标记).
#[derive(Clone)]
struct Conn {
    sysd: watch::Receiver<Option<SysD>>,
    exiting: watch::Receiver<bool>,
}

impl Conn {
    fn new(sysd: watch::Receiver<Option<SysD>>, exiting: watch::Receiver<bool>) -> Self {
        Self { sysd, exiting }
    }
}

/// 处理一个被接受的连接 (应用并发限制后交给 [`handle_conn`]).
///
/// `conn_seq` 为连接序号计数器, 每次接受连接时递增 (从 1 开始).
fn handle_accept(
    accepted: Result<SysdConn, PaError>,
    ro: bool,
    st: Conn,
    sem: &Arc<Semaphore>,
    conn_seq: &mut u64,
    limit: &SysdLimit,
) {
    match accepted {
        Ok(conn) => {
            *conn_seq += 1;
            let seq = *conn_seq;
            trace!("accept #{} ro={}", seq, ro);
            let max = if ro {
                limit.ipc_conn_ro_n
            } else {
                limit.ipc_conn_n
            };
            let permit = match sem.clone().try_acquire_owned() {
                Ok(p) => p,
                Err(_) => {
                    error!("too many concurrent connections (max {})", max);
                    drop(conn);
                    return;
                }
            };
            let limit = limit.clone();
            tokio::spawn(async move {
                let _permit = permit;
                handle_conn(conn, st, ro, seq, limit).await;
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
async fn handle_conn(conn: SysdConn, mut st: Conn, ro: bool, seq: u64, limit: SysdLimit) {
    let _close = ConnGuard { seq, ro };
    let mut conn = conn;

    // 握手: 连接建立后 1 秒内未收到握手消息, 报错并关闭连接.
    match conn.wait_message(limit.ipc_handshake_timeout).await {
        Ok(()) => {}
        Err(PaError::Eof) => return,
        Err(PaError::Timeout(_)) => {
            error!("handshake timeout");
            return;
        }
        Err(e) => {
            error!("handshake recv: {}", e);
            return;
        }
    }

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
        // 空闲超时: 未在等待 SdReq 响应时, 1 秒内没有新请求则关闭连接.
        match conn.wait_message(limit.ipc_idle_timeout).await {
            Ok(()) => {}
            Err(PaError::Eof) => return,
            Err(PaError::Timeout(_)) => {
                debug!("idle timeout, closing connection");
                return;
            }
            Err(e) => {
                error!("recv: {}", e);
                return;
            }
        }

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
            send_conn_resp(&mut conn, &SdRes::Err("read-only connection".to_string())).await;
            return;
        }

        let sysd = {
            let v = st.sysd.borrow_and_update();
            v.clone()
        };

        if is_system_running(&cmd) {
            // 已开始退出: is-system-running 由 Server 直接拦截, 返回 stopping.
            if *st.exiting.borrow() {
                send_conn_resp(&mut conn, &SdRes::SystemState(DaemonSta::Stopping)).await;
                continue;
            }
            match sysd {
                Some(sysd) => forward_cmd(&mut conn, sysd, cmd).await,
                None => {
                    // 启动尚未完成: is-system-running 由 Server 直接回复.
                    send_conn_resp(&mut conn, &SdRes::SystemState(DaemonSta::Initializing)).await;
                    continue;
                }
            }
        } else {
            match sysd {
                Some(sysd) => forward_cmd(&mut conn, sysd, cmd).await,
                None => match wait_sysd(&mut st.sysd).await {
                    Some(sysd) => forward_cmd(&mut conn, sysd, cmd).await,
                    None => {
                        send_conn_resp(&mut conn, &SdRes::Err("sysd stopped".to_string())).await;
                        return;
                    }
                },
            }
        }
    }
}

/// 判断一条请求是否为 `is-system-running`.
fn is_system_running(cmd: &SdReq) -> bool {
    matches!(cmd, SdReq::UnitdRO(SdReqUnitdRO::SystemState))
}

/// 等待 SysD 句柄出现; 若广播通道关闭, 返回 None.
async fn wait_sysd(sysd: &mut watch::Receiver<Option<SysD>>) -> Option<SysD> {
    loop {
        {
            let v = sysd.borrow_and_update();
            if let Some(s) = v.clone() {
                return Some(s);
            }
        }
        if sysd.changed().await.is_err() {
            return None;
        }
    }
}

/// 把一条命令转发给 SysD, 并把响应写回连接.
async fn forward_cmd(conn: &mut SysdConn, sysd: SysD, cmd: SdReq) {
    let system_running = is_system_running(&cmd);

    let (tx, rx) = oneshot::channel();
    if sysd.send(SysDM::SdReq(cmd, tx)).await.is_err() {
        error!("send command to sysd failed (sysd stopped)");
        let resp = if system_running {
            SdRes::SystemState(DaemonSta::Stopping)
        } else {
            SdRes::Err("sysd stopped".to_string())
        };
        send_conn_resp(conn, &resp).await;
        return;
    }

    let resp = rx.await.unwrap_or_else(|_| {
        if system_running {
            SdRes::SystemState(DaemonSta::Stopping)
        } else {
            SdRes::Err("sysd stopped".to_string())
        }
    });
    send_conn_resp(conn, &resp).await;
}

/// 编码并发送一条响应 (发送失败仅记录日志, 由调用方决定是否继续).
async fn send_conn_resp(conn: &mut SysdConn, resp: &SdRes) {
    match encode_frame(resp) {
        Ok(line) => {
            if let Err(e) = conn.send(line).await {
                error!("send response: {}", e);
            }
        }
        Err(e) => {
            error!("encode response: {}", e);
        }
    }
}

/// 独立接受连接的循环: 把接受的连接转发到主循环通道.
///
/// 主循环退出后通道接收端被丢弃, 发送失败即结束本任务.
async fn accept_loop(mut listener: SysdListener, tx: mpsc::Sender<Result<SysdConn, PaError>>) {
    loop {
        match listener.accept().await {
            Ok(conn) => {
                if tx.send(Ok(conn)).await.is_err() {
                    break;
                }
            }
            Err(e) => {
                if tx.send(Err(e)).await.is_err() {
                    break;
                }
            }
        }
    }
}

/// 处理外部优雅退出信号: 交给 SysD 统一处理.
async fn handle_signal(sysd: &watch::Receiver<Option<SysD>>) {
    let sysd = {
        let v = sysd.borrow();
        v.clone()
    };
    match sysd {
        Some(sysd) => {
            let (tx, rx) = oneshot::channel();
            if sysd
                .send(SysDM::SdReq(SdReq::SysD(SdReqSysd::Exit), tx))
                .await
                .is_err()
            {
                error!("send exit signal to sysd failed (sysd stopped)");
                return;
            }
            if rx.await.is_err() {
                error!("exit signal: sysd stopped before reply");
            }
        }
        None => {
            // 启动尚未完成, 无法优雅退出. 此窗口很小, 暂只记录日志.
            error!("exit signal before sysd started");
        }
    }
}
