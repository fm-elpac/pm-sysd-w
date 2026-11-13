//! ServiceD: 管理所有 service (service 集合).
//!
//! ServiceD 是 `tokio::spawn` 的 async task, 内部只持有 [`ServiceDSM`] 状态机
//! (纯计算) 与活跃 ServiceS 的句柄映射. 收到 [`ServiceDM`] 消息后交给状态机决策,
//! 再执行状态机输出的 [`SmActionR`] / [`SmActionD`] 列表.
//!
//! ServiceS 的创建 (新启动) 不直接在主循环中执行, 而是交给一个专用 job 队列
//! (另一个 async task) 串行处理: 主循环只入队创建 job (携带初始消息), job 队列
//! 创建 ServiceS 后无阻塞转发初始消息 (不等待执行完成).

use std::collections::HashMap;

use tokio::sync::{mpsc, oneshot};

use crate::{
    conf::Unit,
    jsonl::{SdReqUnitOp, SdRes},
    logd::Logd,
    sysd::{ServiceSta, UnitSta},
};

use super::{
    SysdError,
    service_s::{ServiceS, ServiceSM, ServiceStatus},
};

mod actor;
mod query;

pub use actor::ServiceOp;

use actor::{ServiceDSM, SmActionD, SmActionR};

/// service 详细状态快照
#[derive(Debug, Clone, Copy)]
pub struct StatusInfo {
    pub active: UnitSta,
    pub sub: ServiceSta,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub exit_status: Option<i32>,
}

/// 单个 unit 操作 (统一结构)
#[derive(Debug, Clone)]
pub struct ServiceCmdUnit {
    /// unit 名称 (ResetFailed 时 "" 表示全部)
    pub name: String,
    /// 当前配置中的 unit 配置 (None = daemon-reload 后已从配置中移除)
    pub conf: Option<Unit>,
    /// 具体操作 (复用 SdReqUnitOp)
    pub op: SdReqUnitOp,
}

/// 发给 ServiceD 的消息 (pub API).
///
/// 每条消息自带 oneshot 回执, 由调用方构造. ServiceD 通过动作执行器回复,
/// 实现无阻塞转发 (参考 UnitD).
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

/// 主循环上下文
#[derive(Clone)]
struct RunCtx {
    job_tx: mpsc::Sender<Job>,
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

/// job 队列回报 (创建完成的 ServiceS)
enum JobReport {
    Started {
        name: String,
        conf: Unit,
        handle: ServiceS,
    },
}

/// ServiceD 句柄
#[derive(Clone)]
pub struct ServiceD {
    tx: mpsc::Sender<ServiceDM>,
}

impl ServiceD {
    /// 启动 ServiceD (含 job 队列), 返回句柄
    pub fn spawn(logd: Logd) -> Self {
        let (tx, rx) = mpsc::channel(64);
        let (job_tx, job_rx) = mpsc::channel(64);
        let (job_report_tx, job_report_rx) = mpsc::channel(64);
        let (svc_report_tx, svc_report_rx) = mpsc::channel(64);

        tokio::spawn(run_job_queue(
            job_rx,
            job_report_tx,
            logd.clone(),
            svc_report_tx.clone(),
        ));

        let rc = RunCtx { job_tx };
        tokio::spawn(async move { run(rx, job_report_rx, svc_report_rx, rc).await });

        Self { tx }
    }

    /// 发送消息 (等待投递完成, 不等待处理/回执)
    pub async fn send(&self, msg: ServiceDM) -> Result<(), mpsc::error::SendError<ServiceDM>> {
        self.tx.send(msg).await
    }
}

/// ServiceD 主循环
async fn run(
    mut rx: mpsc::Receiver<ServiceDM>,
    mut job_report_rx: mpsc::Receiver<JobReport>,
    mut svc_report_rx: mpsc::Receiver<(String, ServiceStatus)>,
    rc: RunCtx,
) {
    let mut sm = ServiceDSM::new();
    let mut handles: HashMap<String, ServiceS> = HashMap::new();

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(msg) = msg else { break };
                match msg {
                    ServiceDM::Unit(cmd, reply) => {
                        let actions = sm.handle_unit(cmd);
                        exec_actions_r(actions, reply, &mut handles, &rc).await;
                    }
                    ServiceDM::StartMany(services, reply) => {
                        let actions = sm.handle_start_many(services);
                        exec_actions_d(actions, reply, &handles, &rc).await;
                    }
                    ServiceDM::Exit(reply) => {
                        let actions = sm.handle_exit();
                        if exec_actions_d(actions, reply, &handles, &rc).await {
                            break;
                        }
                    }
                    ServiceDM::QueryState { name, reply } => {
                        query::spawn_query_state(
                            handles.get(&name).cloned(),
                            sm.last_status(&name),
                            reply,
                        );
                    }
                    ServiceDM::QueryStatus { name, reply } => {
                        query::spawn_query_status(
                            handles.get(&name).cloned(),
                            sm.last_status(&name),
                            reply,
                        );
                    }
                    ServiceDM::QueryStates { names, reply } => {
                        let items = names
                            .into_iter()
                            .map(|name| {
                                let handle = handles.get(&name).cloned();
                                let last = sm.last_status(&name);
                                (name, handle, last)
                            })
                            .collect();
                        query::spawn_query_states(items, reply);
                    }
                    ServiceDM::QueryRunningUnit { name, reply } => {
                        reply
                            .send(sm.running_unit(&name))
                            .expect("service_d query reply channel closed");
                    }
                }
            }
            report = job_report_rx.recv() => {
                let Some(report) = report else { continue };
                match report {
                    JobReport::Started { name, conf, handle } => {
                        handles.insert(name.clone(), handle);
                        sm.started(name, conf);
                    }
                }
            }
            svc_report = svc_report_rx.recv() => {
                let Some((name, status)) = svc_report else { continue };
                handles.remove(&name);
                sm.terminated(&name, status);
            }
        }
    }
}

/// 执行状态机输出的动作列表 (Unit 命令路径)
async fn exec_actions_r(
    actions: Vec<SmActionR>,
    tx: oneshot::Sender<SdRes>,
    handles: &mut HashMap<String, ServiceS>,
    rc: &RunCtx,
) {
    // tx 只能使用一次
    let mut tx = Some(tx);

    for action in actions {
        match action {
            SmActionR::Reply(resp) => {
                tx.take().unwrap().send(resp).unwrap();
            }
            SmActionR::SendServiceS { name, op } => {
                let handle = handles.get(&name).cloned().expect("active service handle");
                let ctl_tx = tx.take().unwrap();
                let (svc_tx, svc_rx) = oneshot::channel();
                handle
                    .send(op.into_sm(svc_tx))
                    .await
                    .expect("send service message");
                spawn_cmd_mapping(svc_rx, ctl_tx);
            }
            SmActionR::SendServiceSAll { op } => {
                let ctl_tx = tx.take().unwrap();
                let handles: Vec<ServiceS> = handles.values().cloned().collect();
                let mut rxs = Vec::with_capacity(handles.len());
                for h in handles {
                    let (svc_tx, svc_rx) = oneshot::channel();
                    h.send(op.into_sm(svc_tx))
                        .await
                        .expect("send service message");
                    rxs.push(svc_rx);
                }
                spawn_cmd_all_mapping(rxs, ctl_tx);
            }
            SmActionR::EnqueueStart { name, conf } => {
                let ctl_tx = tx.take().unwrap();
                let (svc_tx, svc_rx) = oneshot::channel();
                rc.job_tx
                    .send(Job::Create {
                        name,
                        conf,
                        msg: ServiceSM::Cmd(ServiceOp::Start, svc_tx),
                    })
                    .await
                    .unwrap();
                spawn_cmd_mapping(svc_rx, ctl_tx);
            }
        }
    }
}

/// 执行状态机输出的动作列表 (StartMany / Exit 路径)
///
/// 返回 `true` 表示收到 [`SmActionD::Exit`], 主循环应退出.
async fn exec_actions_d(
    actions: Vec<SmActionD>,
    tx: oneshot::Sender<Result<(), SysdError>>,
    handles: &HashMap<String, ServiceS>,
    rc: &RunCtx,
) -> bool {
    // tx 只能使用一次
    let mut tx = Some(tx);

    for action in actions {
        match action {
            SmActionD::Reply(resp) => {
                tx.take().unwrap().send(resp).unwrap();
            }
            SmActionD::SendServiceS { name, op } => {
                let handle = handles.get(&name).cloned().expect("active service handle");
                let (svc_tx, svc_rx) = oneshot::channel();
                handle
                    .send(op.into_sm(svc_tx))
                    .await
                    .expect("send service message");
                spawn_cmd_log(svc_rx, name);
            }
            SmActionD::EnqueueStart { name, conf } => {
                let (svc_tx, svc_rx) = oneshot::channel();
                rc.job_tx
                    .send(Job::Create {
                        name: name.clone(),
                        conf,
                        msg: ServiceSM::Cmd(ServiceOp::Start, svc_tx),
                    })
                    .await
                    .unwrap();
                spawn_cmd_log(svc_rx, name);
            }
            SmActionD::StopAll => {
                stop_all(handles).await;
            }
            SmActionD::Exit => {
                return true;
            }
        }
    }
    false
}

/// 停止所有活跃 service (先并发发送 Stop, 再并发等待回执)
async fn stop_all(handles: &HashMap<String, ServiceS>) {
    let mut rxs = Vec::with_capacity(handles.len());
    for h in handles.values() {
        let (tx, rx) = oneshot::channel();
        h.send(ServiceSM::Cmd(ServiceOp::Stop, tx))
            .await
            .expect("send stop message");
        rxs.push(rx);
    }

    let mut joins = tokio::task::JoinSet::new();
    for rx in rxs {
        joins.spawn(async move {
            rx.await
                .expect("stop_all: service stopped")
                .expect("stop_all: service reply");
        });
    }
    while let Some(r) = joins.join_next().await {
        r.expect("stop_all: wait task");
    }
}

/// 等待一个 ServiceS 命令回执, 映射为 `SdRes` (独立 task, 不阻塞主循环).
fn spawn_cmd_mapping(rx: oneshot::Receiver<Result<(), SysdError>>, tx: oneshot::Sender<SdRes>) {
    tokio::spawn(async move {
        let resp = match rx.await {
            Ok(Ok(())) => SdRes::Ok,
            Ok(Err(e)) => SdRes::Err(e.to_string()),
            Err(_) => SdRes::Err("service stopped".to_string()),
        };
        tx.send(resp).unwrap();
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
            Ok(()) => SdRes::Ok,
            Err(e) => SdRes::Err(e.to_string()),
        };
        tx.send(resp).unwrap();
    });
}

/// 等待一个 ServiceS 命令回执, 失败仅记录日志 (用于批量启动等无需回复场景)
fn spawn_cmd_log(rx: oneshot::Receiver<Result<(), SysdError>>, name: String) {
    tokio::spawn(async move {
        if let Ok(Err(e)) = rx.await {
            log::error!("service `{}` command failed: {}", name, e);
        }
    });
}

/// job 队列: 串行创建 ServiceS (不等待初始消息执行完成)
async fn run_job_queue(
    mut rx: mpsc::Receiver<Job>,
    report_tx: mpsc::Sender<JobReport>,
    logd: Logd,
    svc_report_tx: mpsc::Sender<(String, ServiceStatus)>,
) {
    while let Some(job) = rx.recv().await {
        match job {
            Job::Create { name, conf, msg } => {
                let handle = ServiceS::spawn(conf.clone(), logd.clone(), svc_report_tx.clone());
                handle
                    .send(msg)
                    .await
                    .expect("send initial service message");

                report_tx
                    .send(JobReport::Started { name, conf, handle })
                    .await
                    .expect("job report channel closed");
            }
        }
    }
}
