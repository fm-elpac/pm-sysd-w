//! ServiceS: 管理单个 service (启动, 停止, 重启, reload 等).
//!
//! ServiceS 是 `tokio::spawn` 的 async task, 只管理一个 service 的动态运行状态.
//! 内部只持有 [`ServiceSSM`] 状态机与其它组件的引用: 收到 [`ServiceSM`] 消息后交给
//! 状态机决策, 再执行状态机返回的 [`Action`] 列表. 进程相关逻辑位于 `proc`.

use std::collections::VecDeque;

use tokio::sync::{mpsc, oneshot};

use crate::{
    conf::Unit,
    logd::Logd,
    sysd::{ServiceSta, UnitSta},
};

use super::{SysdError, service_d::ServiceOp};

mod actor;
mod proc;

#[cfg(test)]
mod test;

use actor::{Action, Event, ExitInfo, ServiceSSM};
use proc::WatcherCmd;

pub use actor::ServiceStatus;

/// 命令回执
type CmdReply = oneshot::Sender<Result<(), SysdError>>;

/// 发给 ServiceS 的消息 (pub API).
///
/// 每条消息自带 oneshot 回执, 由调用方构造; ServiceS 处理完后通过该回执回复.
pub enum ServiceSM {
    /// 单个操作 (Start / Stop / ...), 回执为命令结果
    Cmd(ServiceOp, CmdReply),
    /// 查询 active / sub 状态
    QueryState(oneshot::Sender<(UnitSta, ServiceSta)>),
    /// 查询详细状态
    QueryStatus(oneshot::Sender<(UnitSta, ServiceSta, Option<u32>, u32, Option<i32>)>),
}

/// 内部事件 (由子进程监视 / 重启定时器生成)
#[derive(Debug)]
enum Ev {
    /// 子进程已退出
    ChildExited(ExitInfo),
    /// 重启定时器到期
    RestartTimer,
}

/// ServiceS 句柄 (供 ServiceD 发送消息)
#[derive(Clone)]
pub struct ServiceS {
    tx: mpsc::Sender<ServiceSM>,
}

impl ServiceS {
    /// 启动一个 ServiceS, 返回句柄
    pub fn spawn(conf: Unit, logd: Logd, report_tx: mpsc::Sender<(String, ServiceStatus)>) -> Self {
        let (tx, rx) = mpsc::channel::<ServiceSM>(64);
        let (ev_tx, ev_rx) = mpsc::channel::<Ev>(64);
        let rt = Runtime {
            sm: ServiceSSM::new(conf),
            logd,
            report_tx,
            ev_tx,
            kill_tx: None,
            restart_timer: None,
        };
        tokio::spawn(async move { run(rx, ev_rx, rt).await });
        Self { tx }
    }

    /// 发送消息 (等待投递完成, 不等待处理/回执).
    pub async fn send(&self, msg: ServiceSM) -> Result<(), mpsc::error::SendError<ServiceSM>> {
        self.tx.send(msg).await
    }
}

/// 运行中的 ServiceS (tokio task 内部): 只持有 SM 与其它组件的引用.
struct Runtime {
    /// 状态机
    sm: ServiceSSM,
    /// 日志
    logd: Logd,
    /// 上报终止状态到 ServiceD
    report_tx: mpsc::Sender<(String, ServiceStatus)>,
    /// 内部事件通道发送端 (子进程/定时器转发内部事件)
    ev_tx: mpsc::Sender<Ev>,
    /// 子进程控制通道 (请求终止)
    kill_tx: Option<mpsc::Sender<WatcherCmd>>,
    /// 重启定时器 (可取消)
    restart_timer: Option<tokio::task::JoinHandle<()>>,
}

/// ServiceS 主循环 (单线程 actor)
async fn run(mut rx: mpsc::Receiver<ServiceSM>, mut ev_rx: mpsc::Receiver<Ev>, mut rt: Runtime) {
    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                handle_msg(msg, &mut rt).await;
            }
            ev = ev_rx.recv() => {
                let Some(ev) = ev else { continue };
                handle_ev(ev, &mut rt).await;
            }
        }

        if rt.sm.terminated() {
            break;
        }
    }
}

/// 处理一条外部消息
async fn handle_msg(msg: ServiceSM, rt: &mut Runtime) {
    match msg {
        ServiceSM::Cmd(op, reply) => {
            reply.send(rt.command(op).await).unwrap();
        }
        ServiceSM::QueryState(reply) => {
            reply.send(rt.query_state()).unwrap();
        }
        ServiceSM::QueryStatus(reply) => {
            reply.send(rt.query_status()).unwrap();
        }
    }
}

/// 处理一条内部事件
async fn handle_ev(ev: Ev, rt: &mut Runtime) {
    match ev {
        Ev::ChildExited(info) => rt.child_exited(info).await,
        Ev::RestartTimer => rt.restart_timer().await,
    }
}

impl Runtime {
    /// unit 名称
    fn name(&self) -> &str {
        &self.sm.conf().name
    }

    /// 执行一个命令 (转发给状态机, 再执行动作)
    async fn command(&mut self, op: ServiceOp) -> Result<(), SysdError> {
        let actions = self.sm.handle(Event::Cmd(op)).map_err(SysdError::Failed)?;
        self.exec_actions(actions).await
    }

    /// 执行状态机返回的动作 (按顺序, 新动作追加到队列末尾)
    async fn exec_actions(&mut self, actions: Vec<Action>) -> Result<(), SysdError> {
        let mut queue: VecDeque<Action> = actions.into();
        let mut spawn_err: Option<SysdError> = None;

        while let Some(action) = queue.pop_front() {
            match action {
                Action::SpawnChild => {
                    if let Err(e) = self.spawn_child().await {
                        let follow = self
                            .sm
                            .handle(Event::SpawnFailed)
                            .expect("handle spawn-failed event");
                        queue.extend(follow);
                        spawn_err = Some(e);
                    }
                }
                Action::KillChild => {
                    let info = self.kill_child().await?;
                    let follow = self
                        .sm
                        .handle(Event::ChildExited(info))
                        .map_err(SysdError::Failed)?;
                    queue.extend(follow);
                }
                Action::RunExecStop => {
                    self.run_exec_stop().await?;
                }
                Action::RunExecReload => {
                    self.run_exec_reload().await?;
                }
                Action::ScheduleRestart { secs } => {
                    self.schedule_restart(secs);
                }
                Action::CancelRestart => {
                    self.cancel_restart();
                }
                Action::NotifyTerminated { status } => {
                    log::debug!("service `{}` terminated", self.name());
                    self.report_tx
                        .send((self.name().to_string(), status))
                        .await
                        .expect("service status report channel closed");
                }
            }
        }

        match spawn_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// 启动子进程, 并启动退出监视 (事件驱动)
    async fn spawn_child(&mut self) -> Result<(), SysdError> {
        let conf = self.sm.conf().clone();
        let child = proc::spawn_child(&conf, &self.logd)?;
        let pid = child.id();
        self.sm.set_child_pid(pid);
        let (kill_tx, kill_rx) = mpsc::channel(1);
        let (natural_tx, natural_rx) = mpsc::channel(1);
        proc::spawn_watcher(child, natural_tx, kill_rx);
        proc::spawn_forwarder(natural_rx, self.ev_tx.clone());
        self.kill_tx = Some(kill_tx);
        log::debug!("service `{}` spawned child (pid {:?})", self.name(), pid);
        Ok(())
    }

    /// 请求终止子进程, 等待退出信息
    async fn kill_child(&mut self) -> Result<ExitInfo, SysdError> {
        let kill_tx = self
            .kill_tx
            .take()
            .ok_or_else(|| SysdError::Failed("no child running".to_string()))?;
        let (reply_tx, reply_rx) = oneshot::channel();
        kill_tx
            .send(WatcherCmd::Kill(reply_tx))
            .await
            .map_err(|_| SysdError::Failed("child watcher stopped".to_string()))?;
        self.sm.set_child_pid(None);
        reply_rx
            .await
            .map_err(|_| SysdError::Failed("child watcher stopped".to_string()))
    }

    /// 执行 ExecStop (失败仅记录日志, 不中断停止流程)
    async fn run_exec_stop(&mut self) -> Result<(), SysdError> {
        let conf = self.sm.conf().clone();
        if let Some(cmd) = conf.service.exec_stop.clone() {
            if let Err(e) = proc::run_exec(&cmd, &conf, self.name(), &self.logd).await {
                self.logd
                    .write_with("error", Some(self.name()), &format!("ExecStop: {}", e))
                    .expect("write ExecStop error log");
            }
        }
        Ok(())
    }

    /// 执行 ExecReload (失败返回错误)
    async fn run_exec_reload(&mut self) -> Result<(), SysdError> {
        let conf = self.sm.conf().clone();
        let cmd =
            conf.service.exec_reload.clone().ok_or_else(|| {
                SysdError::Failed(format!("Unit {} has no ExecReload.", self.name()))
            })?;
        proc::run_exec(&cmd, &conf, self.name(), &self.logd).await
    }

    /// 安排定时重启
    fn schedule_restart(&mut self, secs: u32) {
        self.cancel_restart();
        let ev_tx = self.ev_tx.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(secs as u64)).await;
            ev_tx
                .send(Ev::RestartTimer)
                .await
                .expect("restart timer channel closed");
        });
        self.restart_timer = Some(handle);
        log::debug!("service `{}` schedule restart in {}s", self.name(), secs);
    }

    /// 取消定时重启
    fn cancel_restart(&mut self) {
        if let Some(h) = self.restart_timer.take() {
            h.abort();
        }
    }

    /// 处理子进程自然退出事件
    async fn child_exited(&mut self, info: ExitInfo) {
        self.sm.set_child_pid(None);
        self.kill_tx = None;
        log::debug!(
            "service `{}` child exited (success {}, code {:?})",
            self.name(),
            info.success,
            info.code
        );
        let actions = self
            .sm
            .handle(Event::ChildExited(info))
            .expect("handle child-exited event");
        self.exec_actions(actions)
            .await
            .expect("execute child-exited actions");
    }

    /// 处理重启定时器事件
    async fn restart_timer(&mut self) {
        self.restart_timer = None;
        let actions = self
            .sm
            .handle(Event::RestartTimer)
            .expect("handle restart-timer event");
        self.exec_actions(actions)
            .await
            .expect("execute restart-timer actions");
    }

    /// 查询 active / sub 状态
    fn query_state(&self) -> (UnitSta, ServiceSta) {
        self.sm.active_sub()
    }

    /// 查询详细状态
    fn query_status(&self) -> (UnitSta, ServiceSta, Option<u32>, u32, Option<i32>) {
        let (active, sub) = self.sm.active_sub();
        (
            active,
            sub,
            self.sm.child_pid(),
            self.sm.restart_count(),
            self.sm.exit_status(),
        )
    }
}
