//! ServiceD actor 外壳 (tokio async task).
//!
//! [`ServiceD`] 是外部句柄 (MPSC 发送端), 外部消息 [`ServiceDM`] 以内部消息 [`M::M`]
//! 包装进入 actor; job 队列创建 ServiceS 完成后通过 [`M::ServiceStarted`] 回报,
//! ServiceS 终止后通过 [`M::ServiceTerminated`] 回报, 停止全部 service 完成后通过
//! [`M::StopAllDone`] 回报. 外部消息与内部事件统一交给 [`Sm`] 状态机决策, 再逐条
//! 执行状态机输出的 [`SmOutput`] 动作. actor 本身不包含业务逻辑, 只执行状态机要求
//! 的 IO 请求.
//!
//! 查询类请求只读取状态: 由 actor 直接组合自己持有的 ServiceS 句柄与状态机持有的
//! 最近终止状态回复, 不经过状态机决策 (不改变状态).

use std::collections::HashMap;

use log::error;
use tokio::sync::{mpsc, oneshot};

use crate::{
    conf::Unit,
    jsonl::SdRes,
    logd::Logd,
    sysd::{
        ServiceSta, SysdError, UnitSta, send_reply,
        service_s::{ServiceS, ServiceSM, ServiceStatus},
    },
};

use super::{
    ServiceCmdUnit, StatusInfo, query,
    sm::{ServiceOp, Sm, SmInput, SmOutput},
};

/// 发给 ServiceD 的外部消息 (pub API).
///
/// 每条消息自带 oneshot 回执, 由调用方构造.
pub enum ServiceDM {
    /// 单个 unit 操作, 回执为 `SdRes`
    Unit(ServiceCmdUnit, oneshot::Sender<SdRes>),
    /// 批量启动 (default.target), 回执为命令结果
    StartMany(Vec<(String, Unit)>, oneshot::Sender<Result<(), SysdError>>),
    /// 停止所有 service 并退出, 回执为命令结果
    Exit(oneshot::Sender<Result<(), SysdError>>),
    /// 查询 active / sub 状态
    QueryState {
        name: String,
        reply: oneshot::Sender<(UnitSta, ServiceSta)>,
    },
    /// 查询详细状态
    QueryStatus {
        name: String,
        reply: oneshot::Sender<StatusInfo>,
    },
    /// 批量查询 active / sub 状态
    QueryStates {
        names: Vec<String>,
        reply: oneshot::Sender<Vec<(String, UnitSta, ServiceSta)>>,
    },
    /// 读取运行中 service 的旧配置
    QueryRunningUnit {
        name: String,
        reply: oneshot::Sender<Option<Unit>>,
    },
}

/// 发给 actor 的内部消息 (外部消息与内部事件共用同一通道)
pub enum M {
    /// 外部消息包装
    M(ServiceDM),
    /// job 队列已创建 ServiceS (携带 name, 配置与句柄)
    ServiceStarted {
        name: String,
        conf: Unit,
        handle: ServiceS,
    },
    /// ServiceS 已终止 (携带 name 与状态)
    ServiceTerminated { name: String, status: ServiceStatus },
    /// 后台 task 已完成停止所有 service (携带结果与原始回执)
    StopAllDone(
        Result<(), SysdError>,
        oneshot::Sender<Result<(), SysdError>>,
    ),
}

/// job 队列中的创建 job
enum Job {
    /// 创建 ServiceS, 并无阻塞转发初始消息 (自带回执)
    Create {
        name: String,
        conf: Unit,
        msg: ServiceSM,
    },
}

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
    /// 启动 ServiceD actor (并启动 job 队列与终止上报转发 task), 返回句柄.
    pub fn spawn(logd: Logd) -> Self {
        let (tx, rx) = mpsc::channel(64);
        let self_tx = tx.clone();

        let (job_tx, job_rx) = mpsc::channel(64);
        let (svc_report_tx, svc_report_rx) = mpsc::channel(64);

        tokio::spawn(run_job_queue(
            job_rx,
            self_tx.clone(),
            logd.clone(),
            svc_report_tx,
        ));
        tokio::spawn(forward_terminated(svc_report_rx, self_tx.clone()));

        tokio::spawn(async move { Actor::new(rx, self_tx.clone(), job_tx).run().await });

        Self { tx }
    }

    /// 发送一条消息 (等待投递完成, 不等待处理/回执)
    pub async fn send(&self, msg: ServiceDM) -> Result<(), mpsc::error::SendError<ServiceDM>> {
        self.tx.send(M::M(msg)).await.map_err(|e| {
            mpsc::error::SendError(match e.0 {
                M::M(m) => m,
                _ => unreachable!("ServiceD::send never sends internal events"),
            })
        })
    }
}

/// actor 本体
pub struct Actor {
    /// 持有状态机
    sm: Sm,
    /// 消息接收端
    rx: mpsc::Receiver<M>,
    /// 消息发送端 (供后台 task 回报停止完成)
    tx: mpsc::Sender<M>,
    /// 活跃 ServiceS 句柄 (状态机不持有句柄, 由 actor 持有)
    handles: HashMap<String, ServiceS>,
    /// job 队列发送端 (创建 ServiceS)
    job_tx: mpsc::Sender<Job>,
    /// 退出标记: 收到 [`SmOutput::Exit`] 后主循环退出
    exit: bool,
}

impl Actor {
    /// 创建 actor
    fn new(rx: mpsc::Receiver<M>, tx: mpsc::Sender<M>, job_tx: mpsc::Sender<Job>) -> Self {
        Self {
            sm: Sm::new(),
            rx,
            tx,
            handles: HashMap::new(),
            job_tx,
            exit: false,
        }
    }

    /// actor 主循环
    pub async fn run(mut self) {
        while let Some(m) = self.rx.recv().await {
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
                M::ServiceStarted { name, conf, handle } => {
                    if self.handles.contains_key(&name) {
                        error!(
                            "service `{}` already running; dropping duplicate handle",
                            name
                        );
                        continue;
                    }
                    let (s1, actions) = self.sm.t(SmInput::ServiceStarted {
                        name: name.clone(),
                        conf,
                    });
                    self.sm = s1;
                    debug_assert!(actions.is_empty());
                    self.handles.insert(name, handle);
                }
                M::ServiceTerminated { name, status } => {
                    self.handles.remove(&name);
                    let (s1, actions) = self.sm.t(SmInput::ServiceTerminated { name, status });
                    self.sm = s1;
                    debug_assert!(actions.is_empty());
                }
                M::StopAllDone(result, reply) => {
                    let (s1, actions) = self.sm.t(SmInput::StopAllDone(result));
                    self.sm = s1;
                    self.exec_actions_d(actions, reply).await;
                }
            }
            if self.exit {
                break;
            }
        }
    }

    /// 执行状态机输出的动作列表 (Unit 命令路径, 回执为 `SdRes`)
    async fn exec_actions_r(&self, actions: Vec<SmOutput>, reply: oneshot::Sender<SdRes>) {
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
                SmOutput::EnqueueStart { name, conf } => {
                    let t = reply.take().unwrap();
                    let (svc_tx, svc_rx) = oneshot::channel();
                    if self
                        .job_tx
                        .send(Job::Create {
                            name,
                            conf,
                            msg: ServiceSM::Cmd(ServiceOp::Start, svc_tx),
                        })
                        .await
                        .is_err()
                    {
                        error!("start job queue stopped");
                        send_reply(t, SdRes::Err("job queue stopped".to_string()));
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
                SmOutput::EnqueueStart { name, conf } => {
                    let (svc_tx, svc_rx) = oneshot::channel();
                    if self
                        .job_tx
                        .send(Job::Create {
                            name: name.clone(),
                            conf,
                            msg: ServiceSM::Cmd(ServiceOp::Start, svc_tx),
                        })
                        .await
                        .is_err()
                    {
                        error!("start job queue stopped");
                    } else {
                        spawn_cmd_log(svc_rx, name);
                    }
                }
                SmOutput::StopAll => {
                    let t = reply.take().unwrap();
                    spawn_stop_all(self.handles.clone(), self.tx.clone(), t);
                }
                SmOutput::Exit => {
                    self.exit = true;
                }
                _ => unreachable!("unexpected ServiceD action for Result reply"),
            }
        }
    }
}

/// 停止所有活跃 service (先并发发送 Stop, 再并发等待回执)
async fn stop_all(handles: &HashMap<String, ServiceS>) {
    let mut rxs = Vec::with_capacity(handles.len());
    for h in handles.values() {
        let (tx, rx) = oneshot::channel();
        if h.send(ServiceSM::Cmd(ServiceOp::Stop, tx)).await.is_err() {
            error!("service stopped before stop command delivery");
        } else {
            rxs.push(rx);
        }
    }

    let mut joins = tokio::task::JoinSet::new();
    for rx in rxs {
        joins.spawn(async move {
            match rx.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => error!("service stop failed: {}", e),
                Err(_) => error!("service stopped before stop reply"),
            }
        });
    }
    while let Some(r) = joins.join_next().await {
        if let Err(e) = r {
            error!("service stop wait task failed: {:?}", e);
        }
    }
}

/// 在后台 task 中停止所有活跃 service, 完成后回报内部事件.
///
/// 该 task 不执行其它退出逻辑; 后续步骤由状态机在收到 [`M::StopAllDone`] 时决定.
fn spawn_stop_all(
    handles: HashMap<String, ServiceS>,
    self_tx: mpsc::Sender<M>,
    reply: oneshot::Sender<Result<(), SysdError>>,
) {
    tokio::spawn(async move {
        stop_all(&handles).await;
        if self_tx.send(M::StopAllDone(Ok(()), reply)).await.is_err() {
            error!("send stop-all-done to ServiceD actor failed (actor stopped)");
        }
    });
}

/// 等待一个 ServiceS 命令回执, 映射为 `SdRes` (独立 task, 不阻塞主循环).
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

/// job 队列: 串行创建 ServiceS (不等待初始消息执行完成)
async fn run_job_queue(
    mut rx: mpsc::Receiver<Job>,
    self_tx: mpsc::Sender<M>,
    logd: Logd,
    svc_report_tx: mpsc::Sender<(String, ServiceStatus)>,
) {
    while let Some(job) = rx.recv().await {
        let Job::Create { name, conf, msg } = job;
        let handle = ServiceS::spawn(conf.clone(), logd.clone(), svc_report_tx.clone());
        if handle.send(msg).await.is_err() {
            error!("service `{}` stopped before initial message", name);
        }

        if self_tx
            .send(M::ServiceStarted { name, conf, handle })
            .await
            .is_err()
        {
            error!("send service-started to ServiceD actor failed (actor stopped)");
            break;
        }
    }
}

/// 把 ServiceS 的终止上报转发为内部事件.
///
/// 该 task 不执行其它逻辑; 后续步骤由状态机在收到 [`M::ServiceTerminated`] 时决定.
async fn forward_terminated(
    mut rx: mpsc::Receiver<(String, ServiceStatus)>,
    self_tx: mpsc::Sender<M>,
) {
    while let Some((name, status)) = rx.recv().await {
        if self_tx
            .send(M::ServiceTerminated { name, status })
            .await
            .is_err()
        {
            error!("send service-terminated to ServiceD actor failed (actor stopped)");
            break;
        }
    }
}
