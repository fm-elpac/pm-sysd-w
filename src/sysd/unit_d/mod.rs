//! UnitD: 管理所有 unit (内存配置) 与 service (通过 ServiceD).
//!
//! UnitD 是 `tokio::spawn` 的 async task. 收到 [`UnitDM`] 消息后交给 [`UnitDSM`]
//! 状态机决策, 再执行状态机输出的 [`SmActionR`] / [`SmActionD`] 列表.
//!
//! UnitS (unit 配置与 enable/mask 状态) 由 UnitDSM 持有; sqlite 数据库仅用于
//! enable/mask 持久化, 启动时读取, 平时通过 WriteDb 动作更新.

use log::error;
use tokio::sync::{mpsc, oneshot};

use crate::{
    conf::{Conf, Unit, UnitType},
    db::DbClient,
    jsonl::{SdReq, SdRes},
    logd::Logd,
};

use super::{
    SysdError,
    service_d::{ServiceD, ServiceDM},
};

mod actor;
mod load;
mod query;
mod unit_s;

use actor::{SmActionD, SmActionR, UnitDSM, WriteUnit};
use load::{load_masked_enabled, reload, write_unit};
use query::spawn_query;
use unit_s::{UnitS, UnitSME};

/// 根据 unit 名称后缀推断类型 (配置中已消失的 unit)
pub fn unit_type_by_name(name: &str) -> Option<UnitType> {
    if name.ends_with(".service") {
        Some(UnitType::Service)
    } else if name.ends_with(".timer") {
        Some(UnitType::Timer)
    } else if name.ends_with(".target") {
        Some(UnitType::Target)
    } else {
        None
    }
}

/// 发给 UnitD 的消息 (pub API).
///
/// 每条消息自带 oneshot 回执, 由调用方构造.
/// UnitD 执行完动作后通过动作执行器回复.
pub enum UnitDM {
    /// ctl 请求 (复用 jsonl 类型), 回执为 `SdRes`
    SdReq(SdReq, oneshot::Sender<SdRes>),
    /// daemon-reload 加载完成 (携带加载结果与原始回执)
    ReloadOk(Conf, oneshot::Sender<SdRes>),
    /// 启动默认 target 中已 enable 的 service
    StartDefaultTarget(oneshot::Sender<Result<(), SysdError>>),
    /// 停止所有 unit, 然后退出本 task 循环
    Exit(oneshot::Sender<Result<(), SysdError>>),
}

/// 主循环上下文
#[derive(Clone)]
pub struct RunCtx {
    pub logd: Logd,
    pub db: DbClient,
    pub conf_path: String,
    pub sd: ServiceD,
    pub self_tx: mpsc::Sender<UnitDM>,
}

/// UnitD 句柄
#[derive(Clone)]
pub struct UnitD {
    tx: mpsc::Sender<UnitDM>,
}

impl UnitD {
    /// 启动 UnitD (并启动其依赖的 ServiceD), 返回句柄
    pub fn spawn(db: DbClient, logd: Logd, conf_path: String, units: Vec<Unit>) -> Self {
        let sd = ServiceD::spawn(logd.clone());

        let (tx, rx) = mpsc::channel(64);
        let self_tx = tx.clone();
        tokio::spawn(async move {
            run(
                rx,
                units,
                RunCtx {
                    logd,
                    db,
                    conf_path,
                    sd,
                    self_tx,
                },
            )
            .await
        });

        Self { tx }
    }

    /// 发送消息 (等待投递完成, 不等待处理/回执)
    pub async fn send(&self, msg: UnitDM) -> Result<(), mpsc::error::SendError<UnitDM>> {
        self.tx.send(msg).await
    }
}

/// UnitD 主循环
async fn run(mut rx: mpsc::Receiver<UnitDM>, units: Vec<Unit>, rc: RunCtx) {
    // 启动时从数据库加载 mask / enable 列表到 UnitS
    let me = match load_masked_enabled(&rc.db).await {
        Ok(v) => v,
        Err(e) => {
            error!("load masked / enabled list: {}", e);
            UnitSME::default()
        }
    };
    let mut sm = UnitDSM::new(UnitS::new(units, me));

    while let Some(msg) = rx.recv().await {
        match msg {
            UnitDM::SdReq(req, reply) => {
                let actions = sm.handle_sd_req(req);
                exec_actions_r(actions, reply, &rc).await;
            }
            UnitDM::ReloadOk(conf, reply) => {
                let actions = sm.handle_reload_ok(conf);
                exec_actions_r(actions, reply, &rc).await;
            }
            UnitDM::StartDefaultTarget(reply) => {
                let actions = sm.handle_start_default_target();
                exec_actions_d(actions, reply, &rc).await;
            }
            UnitDM::Exit(reply) => {
                let actions = sm.handle_exit();
                if exec_actions_d(actions, reply, &rc).await {
                    break;
                }
            }
        }
    }
}

/// 执行状态机输出的动作列表 (StartDefaultTarget / Exit 路径)
///
/// 返回 `true` 表示收到 [`SmActionD::Exit`], 主循环应退出.
async fn exec_actions_d(
    actions: Vec<SmActionD>,
    tx: oneshot::Sender<Result<(), SysdError>>,
    rc: &RunCtx,
) -> bool {
    // tx 只能使用一次
    let mut tx = Some(tx);

    for a in actions {
        match a {
            SmActionD::Log { level, unit, msg } => {
                rc.logd.write_with(level, unit.as_deref(), &msg).unwrap();
            }
            SmActionD::Reply(resp) => {
                tx.take().unwrap().send(resp).unwrap();
            }
            SmActionD::SendStartMany(services) => {
                rc.sd
                    .send(ServiceDM::StartMany(services, tx.take().unwrap()))
                    .await
                    .unwrap();
            }
            SmActionD::ExitService => {
                // 退出是特殊操作: 等待 ServiceD 停止所有 service 完成后再继续
                let (reply, rx) = oneshot::channel();
                rc.sd.send(ServiceDM::Exit(reply)).await.unwrap();
                let r = rx
                    .await
                    .unwrap_or_else(|_| Err(SysdError::Failed("ServiceD stopped".to_string())));
                tx.take().unwrap().send(r).unwrap();
            }
            SmActionD::Exit => {
                return true;
            }
        }
    }
    false
}

/// 执行状态机输出的动作列表 (SdReq 路径)
async fn exec_actions_r(actions: Vec<SmActionR>, tx: oneshot::Sender<SdRes>, rc: &RunCtx) -> bool {
    // tx 只能使用一次
    let mut tx = Some(tx);

    let mut iter = actions.into_iter();

    while let Some(a) = iter.next() {
        match a {
            SmActionR::Log { level, unit, msg } => {
                rc.logd.write_with(level, unit.as_deref(), &msg).unwrap();
            }
            SmActionR::Reply(resp) => {
                tx.take().unwrap().send(resp).unwrap();
            }
            SmActionR::SendSelf(req) => {
                rc.self_tx
                    .send(UnitDM::SdReq(req, tx.take().unwrap()))
                    .await
                    .unwrap();
            }
            SmActionR::SendService(unit) => {
                rc.sd
                    .send(ServiceDM::Unit(unit, tx.take().unwrap()))
                    .await
                    .unwrap();
            }
            // 暂不支持 timer
            // SmActionR::SendTimer => {
            // }
            SmActionR::QueryService(req) => {
                spawn_query(req, rc.sd.clone(), tx.take().unwrap());
            }
            SmActionR::WriteDb(write) => {
                // 写数据库放到独立 task 中, 主循环不等待.
                // 完成后执行剩余动作.
                write_db(write, iter.collect(), tx.take().unwrap(), rc.clone());
                return false;
            }
            SmActionR::Reload => {
                reload(rc.conf_path.clone(), tx.take().unwrap(), rc.self_tx.clone());
            }
        }
    }
    false
}

fn write_db(write: WriteUnit, rest: Vec<SmActionR>, tx: oneshot::Sender<SdRes>, rc: RunCtx) {
    tokio::spawn(async move {
        match write_unit(&rc.db, &write).await {
            Ok(()) => {
                exec_actions_r(rest, tx, &rc).await;
            }
            Err(e) => {
                tx.send(SdRes::Err(e.to_string())).unwrap();
            }
        }
    });
}
