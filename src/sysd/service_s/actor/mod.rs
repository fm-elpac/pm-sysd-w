//! ServiceS actor 外壳 (tokio async task).
//!
//! [`ServiceS`] 是外部句柄 (MPSC 发送端), 外部消息 [`ServiceSM`] 以内部消息 [`M::M`]
//! 包装进入 actor.
//!
//! 子进程创建/退出 / Exec* 完成 / 定时器到期等后台任务通过
//! [`M::Spawned`] / [`M::SpawnFailed`] / [`M::ChildExited`] / [`M::ExecStopDone`] /
//! [`M::ReloadDone`] / [`M::RestartTimer`] / [`M::StartTimeout`] 等内部事件回报.
//!
//! 外部消息与内部事件统一交给 [`Sm`] 状态机决策, actor 只逐条执行状态机输出的
//! [`SmOutput`] 动作, 不包含业务逻辑, 也不反复喂给状态机.
//!
//! 进程启动 / 停止 / Exec* 均为异步: actor 只发起请求并立即返回主循环, 结果通过内部
//! 事件回传, 因此 ExecStart 执行期间主循环不会被阻塞.
//!
//! ServiceS 不自行退出: 状态机需要退出时输出 [`SmOutput::RequestExit`], actor 通过
//! 持有的 [`ServiceD`] 发送退出请求, 待 ServiceD 回发 [`ServiceSM::Exit`] 后退出.

use std::fmt;

use log::{debug, error, trace};
use tokio::sync::{mpsc, oneshot};

use crate::{
    conf::Unit,
    logd::Logd,
    sysd::{
        send_reply,
        service_d::{MS, ServiceD},
    },
    sysu::{ServiceSta, UnitSta},
};

use super::{
    proc,
    sm::{ServiceStatusInfo, Sm, SmInput, SmOutput},
};

mod m;

pub use m::{CmdReply, M, ServiceSM};

/// ServiceS 句柄 (供 ServiceD 发送消息)
#[derive(Clone)]
pub struct ServiceS {
    tx: mpsc::Sender<M>,
    /// unit 名称 (用于 Debug / 日志)
    name: String,
}

impl ServiceS {
    /// 启动一个 ServiceS, 返回句柄.
    ///
    /// `sd` 为 ServiceD 句柄, 供 ServiceS 发送退出请求.
    pub fn spawn(conf: Unit, logd: Logd, sd: ServiceD, channel_capacity_n: usize) -> Self {
        let name = conf.name.clone();
        let (tx, rx) = mpsc::channel(channel_capacity_n);
        let self_tx = tx.clone();

        tokio::spawn(async move { Actor::new(rx, self_tx, conf, logd, sd).run().await });

        Self { tx, name }
    }

    /// 发送一条消息 (等待投递完成, 不等待处理/回执)
    pub async fn send(&self, msg: ServiceSM) -> Result<(), mpsc::error::SendError<ServiceSM>> {
        self.tx.send(M::M(msg)).await.map_err(|e| {
            mpsc::error::SendError(match e.0 {
                M::M(m) => m,
                _ => unreachable!("ServiceS::send never sends internal events"),
            })
        })
    }
}

impl fmt::Debug for ServiceS {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceS")
            .field("name", &self.name)
            .finish()
    }
}

/// actor 本体
pub struct Actor {
    /// 持有状态机
    sm: Sm,
    /// 消息接收端
    rx: mpsc::Receiver<M>,
    /// 消息发送端 (供后台任务回报内部事件)
    tx: mpsc::Sender<M>,
    /// service 配置快照 (供进程 IO 使用, 与状态机各自持有)
    conf: Unit,
    /// Logd 句柄
    logd: Logd,
    /// ServiceD 句柄 (用于发送退出请求)
    sd: ServiceD,
    /// 子进程控制通道 (请求终止)
    kill_tx: Option<mpsc::Sender<proc::WatcherCmd>>,
    /// 重启定时器 (可取消)
    restart_timer: Option<tokio::task::JoinHandle<()>>,
    /// 启动超时定时器 (可取消)
    start_timer: Option<tokio::task::JoinHandle<()>>,
    /// 等待异步完成的命令回执 (同一时刻最多一个)
    pending_cmd: Option<CmdReply>,
    /// 退出标记: 收到 [`ServiceSM::Exit`] 后主循环退出
    exit: bool,
}

impl Actor {
    /// 创建 actor (并构造初始状态机)
    fn new(
        rx: mpsc::Receiver<M>,
        tx: mpsc::Sender<M>,
        conf: Unit,
        logd: Logd,
        sd: ServiceD,
    ) -> Self {
        trace!("{} new", conf.name);

        Self {
            sm: Sm::new(conf.clone()),
            rx,
            tx,
            conf,
            logd,
            sd,
            kill_tx: None,
            restart_timer: None,
            start_timer: None,
            pending_cmd: None,
            exit: false,
        }
    }

    /// actor 主循环
    pub async fn run(mut self) {
        trace!("{} run", self.name());

        while let Some(m) = self.rx.recv().await {
            trace!("{} recv {:?}", self.name(), m);

            match m {
                M::M(ServiceSM::Cmd(op, reply)) => {
                    let (s1, a) = self.sm.t(SmInput::Cmd(op));
                    self.sm = s1;
                    // 同步完成的命令直接回复, 异步命令挂起回执
                    if a.iter().any(|a| matches!(a, SmOutput::ReplyCmd(_))) {
                        self.exec_actions(a, Some(ReplyCmd::Cmd(reply))).await;
                    } else {
                        self.pending_cmd = Some(reply);
                        self.exec_actions(a, None).await;
                    }
                }
                M::M(ServiceSM::QueryState(reply)) => {
                    let (s1, a) = self.sm.t(SmInput::QueryState);
                    self.sm = s1;
                    self.exec_actions(a, Some(ReplyCmd::State(reply))).await;
                }
                M::M(ServiceSM::QueryStatus(reply)) => {
                    let (s1, a) = self.sm.t(SmInput::QueryStatus);
                    self.sm = s1;
                    self.exec_actions(a, Some(ReplyCmd::Status(reply))).await;
                }
                M::M(ServiceSM::Exit) => {
                    self.exit = true;
                }
                M::Spawned { pid } => {
                    let (s1, a) = self.sm.t(SmInput::Spawned { pid });
                    self.sm = s1;
                    self.exec_actions(a, None).await;
                }
                M::SpawnFailed(e) => {
                    let (s1, a) = self.sm.t(SmInput::SpawnFailed(e));
                    self.sm = s1;
                    self.exec_actions(a, None).await;
                }
                M::ChildExited(info) => {
                    self.kill_tx = None;
                    let (s1, a) = self.sm.t(SmInput::ChildExited(info));
                    self.sm = s1;
                    self.exec_actions(a, None).await;
                }
                M::ExecStopDone => {
                    let (s1, a) = self.sm.t(SmInput::ExecStopDone);
                    self.sm = s1;
                    self.exec_actions(a, None).await;
                }
                M::ReloadDone(result) => {
                    let (s1, a) = self.sm.t(SmInput::ReloadDone(result));
                    self.sm = s1;
                    self.exec_actions(a, None).await;
                }
                M::RestartTimer => {
                    self.restart_timer = None;
                    let (s1, a) = self.sm.t(SmInput::RestartTimer);
                    self.sm = s1;
                    self.exec_actions(a, None).await;
                }
                M::StartTimeout => {
                    self.start_timer = None;
                    let (s1, a) = self.sm.t(SmInput::StartTimeout);
                    self.sm = s1;
                    self.exec_actions(a, None).await;
                }
            }

            if self.exit {
                break;
            }
        }

        trace!("{} exit", self.name());
    }

    /// 逐条执行状态机输出的动作.
    ///
    /// `reply` 用于查询类回执; `SmOutput::ReplyCmd` 优先使用传入的 Cmd 回执, 否则
    /// 使用挂起的 [`Actor::pending_cmd`].
    async fn exec_actions(&mut self, actions: Vec<SmOutput>, reply: Option<ReplyCmd>) {
        let mut reply = reply;

        for a in actions {
            match a {
                SmOutput::ReplyCmd(result) => match reply.take() {
                    Some(ReplyCmd::Cmd(t)) => send_reply(t, result),
                    Some(_) => unreachable!("ReplyCmd requires Cmd reply"),
                    None => {
                        let t = self
                            .pending_cmd
                            .take()
                            .expect("ReplyCmd without pending Cmd");
                        send_reply(t, result);
                    }
                },
                SmOutput::ReplyState(state) => {
                    let Some(ReplyCmd::State(t)) = reply.take() else {
                        unreachable!("ReplyState requires state reply");
                    };
                    send_reply(t, state);
                }
                SmOutput::ReplyStatus(status) => {
                    let Some(ReplyCmd::Status(t)) = reply.take() else {
                        unreachable!("ReplyStatus requires status reply");
                    };
                    send_reply(t, status);
                }
                SmOutput::SpawnChild => {
                    self.spawn_child().await;
                }
                SmOutput::KillChild => {
                    self.kill_child().await;
                }
                SmOutput::RunExecStop => {
                    self.run_exec_stop();
                }
                SmOutput::RunExecReload => {
                    self.run_exec_reload();
                }
                SmOutput::ScheduleRestart { secs } => {
                    self.schedule_restart(secs);
                }
                SmOutput::CancelRestart => {
                    self.cancel_restart();
                }
                SmOutput::ScheduleStartTimeout { secs } => {
                    self.schedule_start_timeout(secs);
                }
                SmOutput::CancelStartTimeout => {
                    self.cancel_start_timeout();
                }
                SmOutput::RequestExit { status } => {
                    debug!("service `{}` terminated", self.name());

                    if self
                        .sd
                        .send_s(MS::ExitRequest {
                            name: self.name().to_string(),
                            status,
                        })
                        .await
                        .is_err()
                    {
                        error!(
                            "service `{}` exit request failed (ServiceD stopped)",
                            self.name()
                        );
                    }
                }
            }
        }
    }

    /// unit 名称
    fn name(&self) -> &str {
        &self.conf.name
    }

    /// 启动子进程, 并启动退出监视 (事件驱动)
    async fn spawn_child(&mut self) {
        match proc::spawn_child(&self.conf, &self.logd) {
            Ok(child) => {
                let pid = child.id();
                let (kill_tx, kill_rx) = mpsc::channel(1);
                // 先回报 Spawned 再启动 watcher, 保证 Spawned 先于 ChildExited 入队.
                if self.tx.send(M::Spawned { pid }).await.is_err() {
                    error!(
                        "service `{}` spawn report failed (ServiceS stopped)",
                        self.name()
                    );
                }
                proc::spawn_watcher(child, self.tx.clone(), kill_rx);
                self.kill_tx = Some(kill_tx);
                debug!("service `{}` spawned child (pid {:?})", self.name(), pid);
            }
            Err(e) => {
                if self.tx.send(M::SpawnFailed(e)).await.is_err() {
                    error!(
                        "service `{}` spawn fail report failed (ServiceS stopped)",
                        self.name()
                    );
                }
            }
        }
    }

    /// 请求终止子进程 (退出结果由 watcher 通过内部事件回报)
    async fn kill_child(&mut self) {
        let Some(kill_tx) = self.kill_tx.take() else {
            error!("service `{}` kill child: no child running", self.name());
            return;
        };
        if kill_tx.send(proc::WatcherCmd::Kill).await.is_err() {
            error!("service `{}` kill child: watcher stopped", self.name());
        }
    }

    /// 执行 ExecStop (失败 / 超时仅记录日志, 不中断停止流程)
    fn run_exec_stop(&self) {
        // TODO: sysd 暂只支持一个 ExecStop, 多行语法只取第一行.
        let Some(cmd) = self.conf.service.exec_stop.first().cloned() else {
            error!("service `{}` ExecStop command missing", self.name());
            return;
        };
        let conf = self.conf.clone();
        let logd = self.logd.clone();
        let name = self.name().to_string();
        let secs = conf.service.timeout_stop_sec;
        let tx = self.tx.clone();

        tokio::spawn(async move {
            let fut = proc::run_exec(&cmd, &conf, &name, &logd);
            if secs == 0 {
                if let Err(e) = fut.await {
                    error!("service `{}` ExecStop: {}", name, e);
                }
            } else {
                match tokio::time::timeout(std::time::Duration::from_secs(secs as u64), fut).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => error!("service `{}` ExecStop: {}", name, e),
                    Err(_) => error!("service `{}` ExecStop timed out after {}s", name, secs),
                }
            }
            if tx.send(M::ExecStopDone).await.is_err() {
                error!(
                    "service `{}` ExecStop done report failed (ServiceS stopped)",
                    name
                );
            }
        });
    }

    /// 执行 ExecReload (结果通过内部事件回报)
    fn run_exec_reload(&self) {
        // TODO: sysd 暂只支持一个 ExecReload, 多行语法只取第一行.
        let Some(cmd) = self.conf.service.exec_reload.first().cloned() else {
            error!("service `{}` ExecReload command missing", self.name());
            return;
        };
        let conf = self.conf.clone();
        let logd = self.logd.clone();
        let name = self.name().to_string();
        let tx = self.tx.clone();

        tokio::spawn(async move {
            let result = proc::run_exec(&cmd, &conf, &name, &logd).await;
            if tx.send(M::ReloadDone(result)).await.is_err() {
                error!(
                    "service `{}` ExecReload done report failed (ServiceS stopped)",
                    name
                );
            }
        });
    }

    /// 安排定时重启
    fn schedule_restart(&mut self, secs: u32) {
        self.cancel_restart();
        let tx = self.tx.clone();
        let name = self.name().to_string();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(secs as u64)).await;
            if tx.send(M::RestartTimer).await.is_err() {
                debug!("service `{}` restart timer: ServiceS stopped", name);
            }
        });
        self.restart_timer = Some(handle);
        debug!("service `{}` schedule restart in {}s", self.name(), secs);
    }

    /// 取消定时重启
    fn cancel_restart(&mut self) {
        if let Some(h) = self.restart_timer.take() {
            h.abort();
        }
    }

    /// 安排启动超时 (仅 Type=oneshot)
    fn schedule_start_timeout(&mut self, secs: u32) {
        self.cancel_start_timeout();
        let tx = self.tx.clone();
        let name = self.name().to_string();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(secs as u64)).await;
            if tx.send(M::StartTimeout).await.is_err() {
                debug!("service `{}` start timeout: ServiceS stopped", name);
            }
        });
        self.start_timer = Some(handle);
        debug!(
            "service `{}` schedule start timeout in {}s",
            self.name(),
            secs
        );
    }

    /// 取消启动超时
    fn cancel_start_timeout(&mut self) {
        if let Some(h) = self.start_timer.take() {
            h.abort();
        }
    }
}

impl fmt::Debug for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServiceSActor")
    }
}

/// 查询类命令回执 (Cmd 回执由 [`Actor::pending_cmd`] 单独保存)
enum ReplyCmd {
    Cmd(CmdReply),
    State(oneshot::Sender<(UnitSta, ServiceSta)>),
    Status(oneshot::Sender<ServiceStatusInfo>),
}

#[cfg(test)]
mod test;
