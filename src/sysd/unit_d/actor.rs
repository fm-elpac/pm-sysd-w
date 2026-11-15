//! UnitD actor 外壳 (tokio async task).
//!
//! [`UnitD`] 是外部句柄 (MPSC 发送端), 外部消息 [`UnitDM`] 以内部消息 [`M::M`]
//! 包装进入 actor; daemon-reload / 数据库写入 / ServiceD 退出等待等后台 task 完成
//! 后通过 [`M::ReloadOk`] / [`M::WriteDbDone`] / [`M::ExitServiceDone`] 等内部事件
//! 回报. 外部消息与内部事件统一交给 [`Sm`] 状态机决策, 再逐条执行状态机输出的
//! [`SmOutput`] 动作. actor 本身不包含业务逻辑, 只执行状态机要求的 IO 请求.

use log::error;
use tokio::sync::{mpsc, oneshot};

use crate::{
    conf::{Conf, Unit},
    db::DbClient,
    jsonl::{SdReq, SdRes},
    logd::Logd,
    sysd::{
        SysdError, send_reply,
        service_d::{ServiceD, ServiceDM},
    },
};

use super::{
    load::{load_masked_enabled, spawn_reload, write_unit},
    query::spawn_query,
    sm::{PendingWrite, Sm, SmInput, SmOutput, WriteUnit},
    unit_s::{UnitS, UnitSME},
};

/// 发给 UnitD 的外部消息 (pub API).
///
/// 每条消息自带 oneshot 回执, 由调用方构造.
pub enum UnitDM {
    /// 处理一条 ctl 请求 (直接复用 jsonl 类型), 回执为 `SdRes`
    SdReq(SdReq, oneshot::Sender<SdRes>),
    /// 启动默认 target, 回执为命令结果
    StartDefaultTarget(oneshot::Sender<Result<(), SysdError>>),
    /// 停止所有 unit, 然后退出本 actor 主循环
    Exit(oneshot::Sender<Result<(), SysdError>>),
}

/// 发给 actor 的内部消息 (外部消息与内部事件共用同一通道)
pub enum M {
    /// 外部消息包装
    M(UnitDM),
    /// daemon-reload 加载完成 (携带配置, 从数据库重新读取的 mask/enable 状态, 与原始回执)
    ReloadOk(Conf, UnitSME, oneshot::Sender<SdRes>),
    /// daemon-reload 加载失败 (携带错误消息与原始回执)
    ReloadErr(String, oneshot::Sender<SdRes>),
    /// 数据库写入完成 (携带结果, 对应的后续操作, 与原始回执)
    WriteDbDone(Result<(), SysdError>, PendingWrite, oneshot::Sender<SdRes>),
    /// ServiceD 已停止所有 service (携带结果与原始回执)
    ExitServiceDone(
        Result<(), SysdError>,
        oneshot::Sender<Result<(), SysdError>>,
    ),
}

/// UnitD 句柄
#[derive(Clone)]
pub struct UnitD {
    tx: mpsc::Sender<M>,
}

impl UnitD {
    /// 启动 UnitD actor (并启动其依赖的 ServiceD), 返回句柄.
    pub fn spawn(db: DbClient, logd: Logd, conf_path: String, units: Vec<Unit>) -> Self {
        let sd = ServiceD::spawn(logd.clone());

        let (tx, rx) = mpsc::channel(64);
        let self_tx = tx.clone();

        tokio::spawn(async move {
            // 启动时从数据库加载 mask / enable 列表到 UnitS
            let me = match load_masked_enabled(&db).await {
                Ok(v) => v,
                Err(e) => {
                    error!("load masked / enabled list: {}", e);
                    UnitSME::default()
                }
            };
            Actor::new(rx, self_tx, logd, db, conf_path, sd, units, me)
                .run()
                .await;
        });

        Self { tx }
    }

    /// 发送一条消息 (等待投递完成, 不等待处理/回执)
    pub async fn send(&self, m: UnitDM) -> Result<(), mpsc::error::SendError<UnitDM>> {
        self.tx.send(M::M(m)).await.map_err(|e| {
            mpsc::error::SendError(match e.0 {
                M::M(m) => m,
                _ => unreachable!("UnitD::send never sends internal events"),
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
    /// 消息发送端 (供自己与后台 task 回报内部事件)
    tx: mpsc::Sender<M>,
    /// Logd 句柄
    logd: Logd,
    /// 数据库句柄 (用于 enable/mask 持久化)
    db: DbClient,
    /// 配置文件路径 (用于 daemon-reload)
    conf_path: String,
    /// ServiceD 句柄
    sd: ServiceD,
    /// 退出标记: 收到 [`SmOutput::Exit`] 后主循环退出
    exit: bool,
}

impl Actor {
    /// 创建 actor (并构造初始状态机)
    fn new(
        rx: mpsc::Receiver<M>,
        tx: mpsc::Sender<M>,
        logd: Logd,
        db: DbClient,
        conf_path: String,
        sd: ServiceD,
        units: Vec<Unit>,
        me: UnitSME,
    ) -> Self {
        Self {
            sm: Sm::new(UnitS::new(units, me)),
            rx,
            tx,
            logd,
            db,
            conf_path,
            sd,
            exit: false,
        }
    }

    /// actor 主循环
    pub async fn run(mut self) {
        while let Some(m) = self.rx.recv().await {
            match m {
                M::M(UnitDM::SdReq(req, reply)) => {
                    let (s1, actions) = self.sm.t(SmInput::SdReq(req));
                    self.sm = s1;
                    self.exec_actions_r(actions, reply).await;
                }
                M::M(UnitDM::StartDefaultTarget(reply)) => {
                    let (s1, actions) = self.sm.t(SmInput::StartDefaultTarget);
                    self.sm = s1;
                    self.exec_actions_d(actions, reply).await;
                }
                M::M(UnitDM::Exit(reply)) => {
                    let (s1, actions) = self.sm.t(SmInput::Exit);
                    self.sm = s1;
                    self.exec_actions_d(actions, reply).await;
                }
                M::ReloadOk(conf, me, reply) => {
                    let (s1, actions) = self.sm.t(SmInput::ReloadOk(conf, me));
                    self.sm = s1;
                    self.exec_actions_r(actions, reply).await;
                }
                M::ReloadErr(e, reply) => {
                    let (s1, actions) = self.sm.t(SmInput::ReloadErr(e));
                    self.sm = s1;
                    self.exec_actions_r(actions, reply).await;
                }
                M::WriteDbDone(result, pending, reply) => {
                    let (s1, actions) = self.sm.t(SmInput::WriteDbDone(result, pending));
                    self.sm = s1;
                    self.exec_actions_r(actions, reply).await;
                }
                M::ExitServiceDone(result, reply) => {
                    let (s1, actions) = self.sm.t(SmInput::ExitServiceDone(result));
                    self.sm = s1;
                    self.exec_actions_d(actions, reply).await;
                }
            }
            if self.exit {
                break;
            }
        }
    }

    /// 执行状态机输出的动作列表 (SdReq / reload / WriteDbDone 路径)
    async fn exec_actions_r(&self, actions: Vec<SmOutput>, reply: oneshot::Sender<SdRes>) {
        // reply 只能使用一次, 重复使用说明状态机输出有 BUG
        let mut reply = Some(reply);

        for a in actions {
            let Some(a) = self.exec_action(a).await else {
                continue;
            };
            match a {
                SmOutput::Reply(resp) => {
                    send_reply(reply.take().unwrap(), resp);
                }
                SmOutput::SendSelf(req) => {
                    let t = reply.take().unwrap();
                    if self.tx.send(M::M(UnitDM::SdReq(req, t))).await.is_err() {
                        error!("send self command failed (UnitD stopped)");
                    }
                }
                SmOutput::SendService(cmd) => {
                    let t = reply.take().unwrap();
                    if self.sd.send(ServiceDM::Unit(cmd, t)).await.is_err() {
                        error!("send unit command to ServiceD failed (ServiceD stopped)");
                    }
                }
                // 暂不支持 timer
                // SmOutput::SendTimer => {}
                SmOutput::QueryService(req) => {
                    let t = reply.take().unwrap();
                    spawn_query(req, self.sd.clone(), t);
                }
                SmOutput::WriteDb(write, pending) => {
                    let t = reply.take().unwrap();
                    spawn_write_db(self.db.clone(), self.tx.clone(), write, pending, t);
                }
                SmOutput::Reload => {
                    let t = reply.take().unwrap();
                    spawn_reload(self.db.clone(), self.conf_path.clone(), self.tx.clone(), t);
                }
                _ => unreachable!("unexpected UnitD action for SdRes reply"),
            }
        }
    }

    /// 执行状态机输出的动作列表 (StartDefaultTarget / Exit 路径)
    async fn exec_actions_d(
        &mut self,
        actions: Vec<SmOutput>,
        reply: oneshot::Sender<Result<(), SysdError>>,
    ) {
        // reply 只能使用一次, 重复使用说明状态机输出有 BUG
        let mut reply = Some(reply);

        for a in actions {
            let Some(a) = self.exec_action(a).await else {
                continue;
            };
            match a {
                SmOutput::ReplyResult(resp) => {
                    send_reply(reply.take().unwrap(), resp);
                }
                SmOutput::SendStartMany(services) => {
                    let t = reply.take().unwrap();
                    if self
                        .sd
                        .send(ServiceDM::StartMany(services, t))
                        .await
                        .is_err()
                    {
                        error!("send start-many to ServiceD failed (ServiceD stopped)");
                    }
                }
                SmOutput::ExitService => {
                    let t = reply.take().unwrap();
                    spawn_exit_wait(self.sd.clone(), self.tx.clone(), t);
                }
                SmOutput::Exit => {
                    self.exit = true;
                }
                _ => unreachable!("unexpected UnitD action for Result reply"),
            }
        }
    }

    /// 执行一个不依赖 reply 类型的动作.
    ///
    /// 支持则执行并返回 `None`; 不支持则原样返回, 由调用方处理.
    async fn exec_action(&self, a: SmOutput) -> Option<SmOutput> {
        match a {
            SmOutput::Log { level, unit, msg } => {
                self.logd.write_with(level, unit.as_deref(), &msg).unwrap();
                None
            }
            o => Some(o),
        }
    }
}

/// 向数据库写入 enable / mask 更新, 完成后通过内部事件回报.
///
/// 该 task 不执行其它逻辑; 后续步骤由状态机在收到 [`M::WriteDbDone`] 时决定.
fn spawn_write_db(
    db: DbClient,
    self_tx: mpsc::Sender<M>,
    write: WriteUnit,
    pending: PendingWrite,
    reply: oneshot::Sender<SdRes>,
) {
    tokio::spawn(async move {
        let result = write_unit(&db, &write).await;
        if self_tx
            .send(M::WriteDbDone(result, pending, reply))
            .await
            .is_err()
        {
            error!("send write-db-done to UnitD actor failed (actor stopped)");
        }
    });
}

/// 请求 ServiceD 停止所有 service, 并仅在收到响应后回报内部事件.
///
/// 该 task 不执行其它退出逻辑; 后续步骤由状态机在收到 [`M::ExitServiceDone`] 时决定.
fn spawn_exit_wait(
    sd: ServiceD,
    self_tx: mpsc::Sender<M>,
    reply: oneshot::Sender<Result<(), SysdError>>,
) {
    tokio::spawn(async move {
        let (tx, rx) = oneshot::channel();
        if sd.send(ServiceDM::Exit(tx)).await.is_err() {
            error!("send exit to ServiceD failed (ServiceD stopped)");
        }
        let result = rx
            .await
            .unwrap_or_else(|_| Err(SysdError::Failed("ServiceD stopped".to_string())));
        if self_tx
            .send(M::ExitServiceDone(result, reply))
            .await
            .is_err()
        {
            error!("send exit-service-done to UnitD actor failed (actor stopped)");
        }
    });
}
