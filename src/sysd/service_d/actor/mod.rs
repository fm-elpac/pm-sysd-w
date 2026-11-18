//! ServiceD actor 外壳 (tokio async task).
//!
//! [`ServiceD`] 是外部句柄 (MPSC 发送端), 外部消息 [`ServiceDM`] 以内部消息 [`M::M`]
//! 包装进入 actor, ServiceS 通过 [`M::S`] 回报退出请求.
//!
//! actor 只持有一个 MPSC 接收通道, 直接创建 / 启动 ServiceS,
//! 并统一把外部消息与内部事件交给 [`Sm`] 状态机决策,
//! 再逐条执行状态机输出的 [`SmOutput`] 动作.
//!
//! ServiceS 不自行退出: 需要退出时向 ServiceD 发送 [`MS::ExitRequest`], 由 ServiceD
//! 回发 [`ServiceSM::Exit`].
//!
//! 创建 ServiceS 时, 先同步登记句柄与 active 状态, 再投递初始命令,
//! 消除 "ServiceS 提前退出回报先于句柄登记" 的竞态.
//!
//! 查询类请求只读取状态: 由 actor 直接组合自己持有的 ServiceS 句柄与状态机持有的
//! 最近终止状态回复, 不经过状态机决策 (不改变状态).

use std::{collections::HashMap, fmt};

use log::{error, trace};
use tokio::sync::{mpsc, oneshot};

use crate::{
    conf::Unit,
    jsonl::SdRes,
    logd::Logd,
    sysd::{
        SysdError, send_reply,
        service_s::{ServiceS, ServiceSM},
    },
};

use super::{
    query,
    sm::{ServiceOp, Sm, SmInput, SmOutput},
};

mod m;

pub use m::{M, MS, ServiceDM};

impl ServiceOp {
    /// 构造携带回执的 [`ServiceSM`] 消息
    fn into_sm(self, reply: oneshot::Sender<Result<(), SysdError>>) -> ServiceSM {
        ServiceSM::Cmd(self, reply)
    }
}

/// ServiceD 句柄
#[derive(Clone)]
pub struct ServiceD {
    tx: mpsc::Sender<M>,
}

impl ServiceD {
    /// 启动 ServiceD actor, 返回句柄
    pub fn spawn(logd: Logd) -> Self {
        let (tx, rx) = mpsc::channel(64);
        let self_tx = tx.clone();

        tokio::spawn(async move { Actor::new(rx, self_tx, logd).run().await });

        Self { tx }
    }

    /// 发送一条消息 (等待投递完成, 不等待处理/回执)
    pub async fn send(&self, m: ServiceDM) -> Result<(), mpsc::error::SendError<ServiceDM>> {
        self.tx.send(M::M(m)).await.map_err(|e| {
            mpsc::error::SendError(match e.0 {
                M::M(m) => m,
                _ => unreachable!("ServiceD::send never sends ServiceS messages"),
            })
        })
    }

    /// 发送一条 ServiceS 回报消息 (等待投递完成, 不等待处理/回执)
    pub async fn send_s(&self, m: MS) -> Result<(), mpsc::error::SendError<MS>> {
        self.tx.send(M::S(m)).await.map_err(|e| {
            mpsc::error::SendError(match e.0 {
                M::S(m) => m,
                _ => unreachable!("ServiceD::send_s never sends external messages"),
            })
        })
    }
}

impl fmt::Debug for ServiceD {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServiceD")
    }
}

/// actor 本体
pub struct Actor {
    /// 持有状态机
    sm: Sm,
    /// 消息接收端
    rx: mpsc::Receiver<M>,
    /// 消息发送端 (供构造 ServiceS 持有的 ServiceD 句柄)
    tx: mpsc::Sender<M>,
    /// Logd 句柄 (创建 ServiceS 时传递)
    logd: Logd,
    /// 活跃 ServiceS 句柄 (状态机不持有句柄, 由 actor 持有)
    handles: HashMap<String, ServiceS>,
    /// 等待所有 service 停止后回复 (StopAll 发起时设置)
    pending_stop_all: Option<oneshot::Sender<Result<(), SysdError>>>,
    /// 退出标记: 收到 [`SmOutput::Exit`] 后主循环退出
    exit: bool,
}

impl Actor {
    /// 创建 actor
    fn new(rx: mpsc::Receiver<M>, tx: mpsc::Sender<M>, logd: Logd) -> Self {
        trace!("new");

        Self {
            sm: Sm::new(),
            rx,
            tx,
            logd,
            handles: HashMap::new(),
            pending_stop_all: None,
            exit: false,
        }
    }

    /// actor 主循环
    pub async fn run(mut self) {
        trace!("run");

        while let Some(m) = self.rx.recv().await {
            trace!("recv {:?}", m);

            match m {
                M::M(ServiceDM::Unit(cmd, reply)) => {
                    let (s1, actions) = self.sm.t(SmInput::Unit(cmd));
                    self.sm = s1;
                    self.exec_actions_r(actions, reply).await;
                }
                M::M(ServiceDM::StartMany(services, reply)) => {
                    let (s1, actions) = self.sm.t(SmInput::StartMany(services));
                    self.sm = s1;
                    self.exec_actions_d(actions, reply).await;
                }
                M::M(ServiceDM::Exit(reply)) => {
                    let (s1, actions) = self.sm.t(SmInput::Exit);
                    self.sm = s1;
                    self.exec_actions_d(actions, reply).await;
                }
                M::M(ServiceDM::QueryState { name, reply }) => {
                    query::spawn_query_state(
                        self.handles.get(&name).cloned(),
                        self.sm.last_status(&name),
                        reply,
                    );
                }
                M::M(ServiceDM::QueryStatus { name, reply }) => {
                    query::spawn_query_status(
                        self.handles.get(&name).cloned(),
                        self.sm.last_status(&name),
                        reply,
                    );
                }
                M::M(ServiceDM::QueryStates { names, reply }) => {
                    let items = names
                        .into_iter()
                        .map(|name| {
                            let handle = self.handles.get(&name).cloned();
                            let last = self.sm.last_status(&name);
                            (name, handle, last)
                        })
                        .collect();
                    query::spawn_query_states(items, reply);
                }
                M::M(ServiceDM::QueryRunningUnit { name, reply }) => {
                    send_reply(reply, self.sm.running_unit(&name));
                }
                M::M(ServiceDM::QueryRunningUnits { reply }) => {
                    send_reply(reply, self.sm.running_units());
                }
                M::S(MS::ExitRequest { name, status }) => {
                    let handle = self.handles.remove(&name);
                    let (s1, actions) = self.sm.t(SmInput::ServiceTerminated {
                        name: name.clone(),
                        status,
                    });
                    self.sm = s1;
                    debug_assert!(actions.is_empty());
                    match handle {
                        Some(h) => {
                            if h.send(ServiceSM::Exit).await.is_err() {
                                error!("service `{}` stopped before exit command delivery", name);
                            }
                        }
                        None => {
                            error!("service `{}` exit request without handle", name);
                        }
                    }
                    self.maybe_finish_stop_all().await;
                }
            }
            if self.exit {
                break;
            }
        }

        trace!("exit");
    }

    /// 执行状态机输出的动作列表 (Unit 命令路径, 回执为 `SdRes`)
    async fn exec_actions_r(&mut self, actions: Vec<SmOutput>, reply: oneshot::Sender<SdRes>) {
        // reply 只能使用一次, 重复使用说明状态机输出有 BUG
        let mut reply = Some(reply);

        for a in actions {
            match a {
                SmOutput::Reply(resp) => {
                    send_reply(reply.take().unwrap(), resp);
                }
                SmOutput::SendServiceS { name, op } => {
                    let t = reply.take().unwrap();
                    match self.handles.get(&name) {
                        Some(handle) => {
                            let (svc_tx, svc_rx) = oneshot::channel();
                            if handle.send(op.into_sm(svc_tx)).await.is_err() {
                                error!("service `{}` stopped before command delivery", name);
                                send_reply(t, SdRes::Err("service stopped".to_string()));
                            } else {
                                spawn_cmd_mapping(svc_rx, t);
                            }
                        }
                        None => {
                            error!("service `{}` not running", name);
                            send_reply(t, SdRes::Err(format!("Unit {} not running.", name)));
                        }
                    }
                }
                SmOutput::SendServiceSAll { op } => {
                    let t = reply.take().unwrap();
                    let mut rxs = Vec::with_capacity(self.handles.len());
                    for h in self.handles.values() {
                        let (svc_tx, svc_rx) = oneshot::channel();
                        if h.send(op.into_sm(svc_tx)).await.is_err() {
                            error!("service stopped before broadcast command delivery");
                        } else {
                            rxs.push(svc_rx);
                        }
                    }
                    spawn_cmd_all_mapping(rxs, t);
                }
                SmOutput::StartService { name, conf } => {
                    let t = reply.take().unwrap();
                    let handle = self.spawn_service(name.clone(), conf);
                    let (svc_tx, svc_rx) = oneshot::channel();
                    if handle
                        .send(ServiceSM::Cmd(ServiceOp::Start, svc_tx))
                        .await
                        .is_err()
                    {
                        error!("service `{}` stopped before start command delivery", name);
                        send_reply(t, SdRes::Err("service stopped".to_string()));
                    } else {
                        spawn_cmd_mapping(svc_rx, t);
                    }
                }
                _ => unreachable!("unexpected ServiceD action for SdRes reply"),
            }
        }
    }

    /// 执行状态机输出的动作列表 (StartMany / Exit 路径, 回执为命令结果)
    async fn exec_actions_d(
        &mut self,
        actions: Vec<SmOutput>,
        reply: oneshot::Sender<Result<(), SysdError>>,
    ) {
        // reply 只能使用一次, 重复使用说明状态机输出有 BUG
        let mut reply = Some(reply);

        for a in actions {
            match a {
                SmOutput::ReplyResult(resp) => {
                    send_reply(reply.take().unwrap(), resp);
                }
                SmOutput::SendServiceS { name, op } => match self.handles.get(&name) {
                    Some(handle) => {
                        let (svc_tx, svc_rx) = oneshot::channel();
                        if handle.send(op.into_sm(svc_tx)).await.is_err() {
                            error!("service `{}` stopped before command delivery", name);
                        } else {
                            spawn_cmd_log(svc_rx, name);
                        }
                    }
                    None => {
                        error!("service `{}` not running", name);
                    }
                },
                SmOutput::StartService { name, conf } => {
                    let handle = self.spawn_service(name.clone(), conf);
                    let (svc_tx, svc_rx) = oneshot::channel();
                    if handle
                        .send(ServiceSM::Cmd(ServiceOp::Start, svc_tx))
                        .await
                        .is_err()
                    {
                        error!("service `{}` stopped before start command delivery", name);
                    } else {
                        spawn_cmd_log(svc_rx, name);
                    }
                }
                SmOutput::StopAll => {
                    let t = reply.take().unwrap();
                    self.begin_stop_all(t).await;
                }
                SmOutput::Exit => {
                    self.exit = true;
                }
                _ => unreachable!("unexpected ServiceD action for Result reply"),
            }
        }
    }

    /// 直接创建并登记一个 ServiceS, 返回句柄.
    ///
    /// 先同步登记句柄与状态机 active 状态, 再让调用方投递初始命令, 保证 ServiceS
    /// 的退出请求不会先于句柄登记到达.
    fn spawn_service(&mut self, name: String, conf: Unit) -> ServiceS {
        let sd = ServiceD {
            tx: self.tx.clone(),
        };
        let handle = ServiceS::spawn(conf.clone(), self.logd.clone(), sd);
        let (s1, actions) = self.sm.clone().t(SmInput::ServiceStarted {
            name: name.clone(),
            conf,
        });
        self.sm = s1;
        debug_assert!(actions.is_empty());
        self.handles.insert(name, handle.clone());
        handle
    }

    /// 发起停止所有 service, 等待所有停止命令回执.
    /// 最终完成由 [`Self::maybe_finish_stop_all`] 在所有句柄移除后触发.
    async fn begin_stop_all(&mut self, reply: oneshot::Sender<Result<(), SysdError>>) {
        if self.handles.is_empty() {
            let (s1, actions) = self.sm.clone().t(SmInput::StopAllDone(Ok(())));
            self.sm = s1;
            self.finish_stop_all_done(actions, reply).await;
            return;
        }

        self.pending_stop_all = Some(reply);

        let handles: Vec<ServiceS> = self.handles.values().cloned().collect();
        let mut rxs = Vec::with_capacity(handles.len());
        for h in &handles {
            let (tx, rx) = oneshot::channel();
            if h.send(ServiceSM::Cmd(ServiceOp::Stop, tx)).await.is_err() {
                error!("service stopped before stop command delivery");
            } else {
                rxs.push(rx);
            }
        }
        for rx in rxs {
            match rx.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => error!("service stop failed: {}", e),
                Err(_) => error!("service stopped before stop reply"),
            }
        }
    }

    /// 执行 `StopAllDone` 的动作 (仅 ReplyResult 与 Exit, 不回到普通动作分发)
    async fn finish_stop_all_done(
        &mut self,
        actions: Vec<SmOutput>,
        reply: oneshot::Sender<Result<(), SysdError>>,
    ) {
        let mut reply = Some(reply);
        for a in actions {
            match a {
                SmOutput::ReplyResult(resp) => {
                    send_reply(reply.take().unwrap(), resp);
                }
                SmOutput::Exit => {
                    self.exit = true;
                }
                _ => unreachable!("unexpected ServiceD action for stop-all-done"),
            }
        }
    }

    /// 所有句柄都已移除且正在等待停止完成时, 推进退出序列最后一步
    async fn maybe_finish_stop_all(&mut self) {
        if self.pending_stop_all.is_none() || !self.handles.is_empty() {
            return;
        }
        let reply = self.pending_stop_all.take().unwrap();
        let (s1, actions) = self.sm.clone().t(SmInput::StopAllDone(Ok(())));
        self.sm = s1;
        self.finish_stop_all_done(actions, reply).await;
    }
}

impl fmt::Debug for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServiceDActor")
    }
}

/// 等待一个 ServiceS 命令回执, 映射为 `SdRes` (独立 task, 不阻塞主循环)
fn spawn_cmd_mapping(rx: oneshot::Receiver<Result<(), SysdError>>, tx: oneshot::Sender<SdRes>) {
    tokio::spawn(async move {
        let resp = match rx.await {
            Ok(Ok(())) => SdRes::Ok(None),
            Ok(Err(e)) => SdRes::Err(e.to_string()),
            Err(_) => SdRes::Err("service stopped".to_string()),
        };
        send_reply(tx, resp);
    });
}

/// 等待全部 ServiceS 命令回执, 映射为 `SdRes`
fn spawn_cmd_all_mapping(
    rxs: Vec<oneshot::Receiver<Result<(), SysdError>>>,
    tx: oneshot::Sender<SdRes>,
) {
    tokio::spawn(async move {
        let mut result: Result<(), SysdError> = Ok(());
        for rx in rxs {
            if let Ok(Err(e)) = rx.await {
                if result.is_ok() {
                    result = Err(e);
                }
            }
        }
        let resp = match result {
            Ok(()) => SdRes::Ok(None),
            Err(e) => SdRes::Err(e.to_string()),
        };
        send_reply(tx, resp);
    });
}

/// 等待一个 ServiceS 命令回执, 失败仅记录日志 (用于批量启动等无需回复场景)
fn spawn_cmd_log(rx: oneshot::Receiver<Result<(), SysdError>>, name: String) {
    tokio::spawn(async move {
        if let Ok(Err(e)) = rx.await {
            error!("service `{}` command failed: {}", name, e);
        }
    });
}

#[cfg(test)]
mod test;
