//! UnitD actor 外壳 (tokio async task).
//!
//! [`UnitD`] 是外部句柄 (MPSC 发送端), 外部消息 [`UnitDM`] 以内部消息 [`M::M`]
//! 包装进入 actor.
//!
//! daemon-reload / 数据库写入 / ServiceD 退出等待等后台 task 完成后通过
//! [`M::ReloadOk`] / [`M::WriteDbDone`] / [`M::ExitServiceDone`]
//! 等内部事件回报.
//!
//! 外部消息与内部事件统一交给 [`Sm`] 状态机决策, 再逐条执行状态机输出的
//! [`SmOutput`] 动作. actor 本身不包含业务逻辑, 只执行状态机要求的 IO 请求.
//!
//! 自身串行操作 (enable / mask / daemon-reload 等) 由 actor 持有的 FIFO 等候队列
//! 排队: 状态机输出 [`SmOutput::Wait`] 时入队, 输出 [`SmOutput::Idle`] 后由 actor
//! 喂入下一条. 转发 ServiceD 的操作不经过该队列.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use log::{error, trace};
use tokio::sync::{mpsc, oneshot};

use crate::conf::limit::SysdLimit;

use crate::{
    db::{DbClient, DbError, UnitState},
    jsonl::{SdReq, SdRes},
    logd::Logd,
    sysd::{
        SysdError,
        reload::{ReloadR, Reloader},
        send_reply,
        service_d::{ServiceD, ServiceDM},
    },
};

use super::{
    query::spawn_query,
    sm::{PendingWrite, Sm, SmInput, SmOutput, WriteUnit},
    unit_s::UnitS,
};

mod m;

pub use m::{M, UnitDM};

/// UnitD 句柄
#[derive(Clone)]
pub struct UnitD {
    tx: mpsc::Sender<M>,
}

impl UnitD {
    /// 启动 UnitD actor (并启动其依赖的 ServiceD), 返回句柄.
    ///
    /// `r` 为已完成的加载结果 (配置与 mask/enable 状态), 直接用于构造初始状态.
    pub fn spawn(
        db: DbClient,
        logd: Logd,
        conf_path: String,
        r: ReloadR,
        reloader: Box<dyn Reloader>,
        limit: &SysdLimit,
    ) -> Self {
        let sd = ServiceD::spawn(logd.clone(), limit.channel_capacity_n);
        let reloader = Arc::from(reloader);

        let (tx, rx) = mpsc::channel(limit.channel_capacity_n);
        let self_tx = tx.clone();
        let ipc_m_byte = limit.ipc_m_byte;

        tokio::spawn(async move {
            Actor::new(
                rx, self_tx, logd, db, conf_path, sd, r, reloader, ipc_m_byte,
            )
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

impl fmt::Debug for UnitD {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UnitD")
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
    /// ServiceD 句柄
    sd: ServiceD,
    /// 配置加载器 (daemon-reload 后台任务使用)
    reloader: Arc<dyn Reloader>,
    /// IPC 单条消息最大长度 (list-units 响应截断)
    ipc_m_byte: usize,
    /// 自身串行操作的等候队列 (FIFO)
    wait: VecDeque<(SdReq, oneshot::Sender<SdRes>)>,
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
        r: ReloadR,
        reloader: Arc<dyn Reloader>,
        ipc_m_byte: usize,
    ) -> Self {
        trace!("new");

        let ReloadR { conf, me } = r;
        Self {
            sm: Sm::new(UnitS::new(conf.units, me), conf_path),
            rx,
            tx,
            logd,
            db,
            sd,
            reloader,
            ipc_m_byte,
            wait: VecDeque::new(),
            exit: false,
        }
    }

    /// actor 主循环
    pub async fn run(mut self) {
        trace!("run");

        while let Some(m) = self.rx.recv().await {
            trace!("recv {:?}", m);

            match m {
                M::M(UnitDM::SdReq(req, reply)) => {
                    let a = self.sm.t(SmInput::SdReq(req.clone()));
                    if a.iter().any(|a| matches!(a, SmOutput::Wait)) {
                        self.wait.push_back((req, reply));
                    } else {
                        self.exec_actions_r(a, reply).await;
                    }
                }
                M::M(UnitDM::StartDefaultTarget(reply)) => {
                    let a = self.sm.t(SmInput::StartDefaultTarget);
                    self.exec_actions_d(a, reply).await;
                }
                M::M(UnitDM::Exit(reply)) => {
                    self.reject_wait();
                    let a = self.sm.t(SmInput::Exit);
                    self.exec_actions_d(a, reply).await;
                }
                M::ReloadOk(r, reply) => {
                    let a = self.sm.t(SmInput::ReloadOk(r));
                    self.exec_actions_r(a, reply).await;
                }
                M::ReloadErr(e, reply) => {
                    let a = self.sm.t(SmInput::ReloadErr(e));
                    self.exec_actions_r(a, reply).await;
                }
                M::WriteDbDone(result, pending, reply) => {
                    let a = self.sm.t(SmInput::WriteDbDone(result, pending));
                    self.exec_actions_r(a, reply).await;
                }
                M::ExitServiceDone(result, reply) => {
                    let a = self.sm.t(SmInput::ExitServiceDone(result));
                    self.exec_actions_d(a, reply).await;
                }
            }
            if self.exit {
                break;
            }
            self.pump_wait().await;
        }

        trace!("exit");
    }

    /// 退出时拒绝等候队列中尚未处理的操作
    fn reject_wait(&mut self) {
        while let Some((_req, reply)) = self.wait.pop_front() {
            send_reply(reply, SdRes::Err("unitd exiting".to_string()));
        }
    }

    /// 状态机空闲时, 依次喂入等候队列中的下一个自身串行操作
    async fn pump_wait(&mut self) {
        while !self.sm.is_busy() {
            let Some((req, reply)) = self.wait.pop_front() else {
                break;
            };
            let a = self.sm.t(SmInput::SdReq(req));
            if a.iter().any(|a| matches!(a, SmOutput::Wait)) {
                unreachable!("state machine Wait while idle");
            }
            self.exec_actions_r(a, reply).await;
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
                SmOutput::Idle => {
                    // 仅作为状态机完成操作的信号, 由主循环 `pump_wait` 喂入下一个操作
                }
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
                    spawn_query(req, self.sd.clone(), t, self.ipc_m_byte);
                }
                SmOutput::WriteDb(write, pending) => {
                    let t = reply.take().unwrap();
                    spawn_write_db(self.db.clone(), self.tx.clone(), write, pending, t);
                }
                SmOutput::Reload(conf_path) => {
                    let t = reply.take().unwrap();
                    spawn_reload(
                        self.db.clone(),
                        self.reloader.clone(),
                        conf_path,
                        self.tx.clone(),
                        t,
                    );
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
    /// 支持则执行并返回 `None`. 不支持则原样返回, 由调用方处理.
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

impl fmt::Debug for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UnitDActor")
    }
}

/// 触发 daemon-reload (spawn 独立 task, 主循环不等待).
///
/// 通过注入的 [`Reloader`] 加载配置 (传入已打开的 `db`, 跳过打开数据库).
/// 成功后回传 [`M::ReloadOk`] (由状态机更新状态并回复).
/// 失败则回传 [`M::ReloadErr`] (由状态机清理 reload 状态并回复, 旧配置保持不变).
fn spawn_reload(
    db: DbClient,
    reloader: Arc<dyn Reloader>,
    conf_path: String,
    tx: mpsc::Sender<M>,
    reply: oneshot::Sender<SdRes>,
) {
    tokio::spawn(async move {
        match reloader.reload(Some(db), &conf_path).await {
            Ok((r, _)) => {
                if tx.send(M::ReloadOk(r, reply)).await.is_err() {
                    error!("reload: UnitD stopped");
                }
            }
            Err(e) => {
                if tx.send(M::ReloadErr(e.to_string(), reply)).await.is_err() {
                    error!("reload: UnitD stopped");
                }
            }
        }
    });
}

/// 向数据库写入 enable / mask 完整状态, 完成后通过内部事件回报.
///
/// 此 task 不执行其它逻辑. 后续步骤由状态机在收到 [`M::WriteDbDone`] 时决定.
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

/// 转换 db 错误
fn db_err(e: DbError) -> SysdError {
    SysdError::Failed(e.to_string())
}

/// 向数据库写入一个 unit 的完整  mask / enable 状态
async fn write_unit(db: &DbClient, w: &WriteUnit) -> Result<(), SysdError> {
    let state = UnitState {
        name: w.name.clone(),
        enabled: w.enabled,
        masked: w.masked,
    };
    db.unit_write(&state).await.map_err(db_err)
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

#[cfg(test)]
mod test;
