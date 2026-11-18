//! SysD actor 外壳 (tokio async task).
//!
//! [`SysD`] 是外部句柄 (MPSC 发送端), 外部消息 [`SysDM`] 以内部消息 [`M::M`]
//! 包装进入 actor; 退出序列中由后台 task 等待 UnitD 响应后通过
//! [`M::ExitUnitsDone`] 回报. 外部消息与内部事件统一交给 [`Sm`] 状态机决策,
//! 再逐条执行状态机输出的 [`SmOutput`] 动作. actor 本身不包含业务逻辑,
//! 只执行状态机要求的 IO 请求.

use std::fmt;

use log::{error, trace};
use tokio::sync::{mpsc, oneshot};

use crate::{
    db::DbClient,
    jsonl::SdRes,
    logd::Logd,
    sysd::{
        SysdError,
        init::Shutdown,
        reload::ReloadR,
        send_reply,
        unit_d::{UnitD, UnitDM},
    },
};

use super::sm::{Sm, SmInput, SmOutput};

mod m;

pub use m::{M, SysDM};

/// SysD 句柄 (整个 pm-sysd-init 管理实例)
#[derive(Clone)]
pub struct SysD {
    tx: mpsc::Sender<M>,
}

impl SysD {
    /// 启动 SysD actor (并启动其依赖的 UnitD), 返回句柄.
    ///
    /// `exit_tx` 用于 SysD 决定退出时通知 `sysd::init`.
    pub fn spawn(
        db: DbClient,
        logd: Logd,
        conf_path: &str,
        r: ReloadR,
        exit_tx: oneshot::Sender<Shutdown>,
    ) -> Self {
        let ud = UnitD::spawn(db.clone(), logd.clone(), conf_path.to_string(), r);

        let (tx, rx) = mpsc::channel(64);

        tokio::spawn(Actor::new(rx, tx.clone(), ud, logd, db, exit_tx).run());

        Self { tx }
    }

    /// 发送一条消息 (等待投递完成, 不等待处理/回执)
    pub async fn send(&self, m: SysDM) -> Result<(), mpsc::error::SendError<SysDM>> {
        self.tx.send(M::M(m)).await.map_err(|e| {
            mpsc::error::SendError(match e.0 {
                M::M(m) => m,
                M::ExitUnitsDone(..) => unreachable!("SysD::send never sends internal events"),
            })
        })
    }
}

impl fmt::Debug for SysD {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SysD")
    }
}

/// actor 本体
pub struct Actor {
    /// 持有状态机
    sm: Sm,
    /// 消息接收端
    rx: mpsc::Receiver<M>,
    /// 消息发送端 (供自己与等待 UnitD 响应的后台 task 回报内部事件)
    tx: mpsc::Sender<M>,
    /// UnitD 句柄
    ud: UnitD,
    /// Logd 句柄
    logd: Logd,
    /// 数据库句柄 (用于 sqlite-vacuum)
    db: DbClient,
    /// 通知 `sysd::init` 退出的发送端 ([`SmOutput::Finish`] 动作消费一次).
    /// 同时作为退出标记, 如果为 `None` 表示应该退出.
    exit_tx: Option<oneshot::Sender<Shutdown>>,
}

impl Actor {
    /// 创建 actor
    pub fn new(
        rx: mpsc::Receiver<M>,
        tx: mpsc::Sender<M>,
        ud: UnitD,
        logd: Logd,
        db: DbClient,
        exit_tx: oneshot::Sender<Shutdown>,
    ) -> Self {
        trace!("new");
        Self {
            sm: Sm::new(),
            rx,
            tx,
            ud,
            logd,
            db,
            exit_tx: Some(exit_tx),
        }
    }

    /// actor 主循环
    pub async fn run(mut self) {
        trace!("run");
        while let Some(m) = self.rx.recv().await {
            trace!("recv {:?}", m);
            match m {
                M::M(SysDM::SdReq(req, reply)) => {
                    let (s1, actions) = self.sm.t(SmInput::SdReq(req));
                    self.sm = s1;
                    self.exec_actions_r(actions, reply).await;
                }
                M::M(SysDM::StartDefaultTarget(reply)) => {
                    let (s1, actions) = self.sm.t(SmInput::StartDefaultTarget);
                    self.sm = s1;
                    self.exec_actions_d(actions, reply).await;
                }
                M::ExitUnitsDone(result, reply) => {
                    let (s1, actions) = self.sm.t(SmInput::ExitUnitsDone(result));
                    self.sm = s1;
                    self.exec_actions_r(actions, reply).await;
                }
            };
            if self.exit_tx.is_none() {
                break;
            }
        }
        trace!("exit");
    }

    /// 执行一个不依赖 reply 类型的动作.
    ///
    /// 支持则执行并返回 `None`; 不支持则原样返回, 由调用方处理.
    async fn exec_action(&mut self, a: SmOutput) -> Option<SmOutput> {
        match a {
            SmOutput::Log { level, msg } => {
                self.logd.write_with(level, None, &msg).unwrap();
                None
            }
            SmOutput::Console { msg } => {
                println!("{}", msg);
                None
            }
            SmOutput::Finish { shutdown } => {
                let t = self.exit_tx.take().unwrap();
                send_reply(t, shutdown);
                None
            }
            o => Some(o),
        }
    }

    /// 执行状态机输出的动作列表 (SdReq 与退出序列路径).
    ///
    /// 返回 `true` 表示收到 [`SmOutput::Finish`], 主循环应退出.
    async fn exec_actions_r(&mut self, actions: Vec<SmOutput>, reply: oneshot::Sender<SdRes>) {
        let mut reply = Some(reply);

        for a in actions {
            let Some(a) = self.exec_action(a).await else {
                continue;
            };
            match a {
                SmOutput::Reply(resp) => {
                    send_reply(reply.take().unwrap(), resp);
                }
                SmOutput::SendLog(log_req) => {
                    let t = reply.take().unwrap();
                    if self.logd.req_log_ro(log_req, t).await.is_err() {
                        error!("send log query to Logd failed (Logd stopped)");
                    }
                }
                SmOutput::SendUnitd(req) => {
                    let t = reply.take().unwrap();
                    if self.ud.send(UnitDM::SdReq(req, t)).await.is_err() {
                        error!("send request to UnitD failed (UnitD stopped)");
                    }
                }
                SmOutput::SqliteVacuum => {
                    let t = reply.take().unwrap();
                    let db = self.db.clone();
                    tokio::spawn(async move {
                        let resp = match db.vacuum().await {
                            Ok(()) => SdRes::Ok(None),
                            Err(e) => SdRes::Err(e.to_string()),
                        };
                        send_reply(t, resp);
                    });
                }
                SmOutput::SendExitUnitd => {
                    let t = reply.take().unwrap();
                    spawn_exit_wait(self.ud.clone(), self.tx.clone(), t);
                }
                _ => unreachable!("unexpected SysD action for SdRes reply"),
            }
        }
    }

    /// 执行状态机输出的动作列表 (StartDefaultTarget 路径).
    ///
    /// 返回 `true` 表示收到 [`SmOutput::Finish`], 主循环应退出.
    async fn exec_actions_d(
        &mut self,
        actions: Vec<SmOutput>,
        reply: oneshot::Sender<Result<(), SysdError>>,
    ) {
        let mut reply = Some(reply);

        for a in actions {
            let Some(a) = self.exec_action(a).await else {
                continue;
            };
            match a {
                SmOutput::ReplyResult(resp) => {
                    send_reply(reply.take().unwrap(), resp);
                }
                SmOutput::SendStartDefaultTarget => {
                    let t = reply.take().unwrap();
                    if self.ud.send(UnitDM::StartDefaultTarget(t)).await.is_err() {
                        error!("send start-default-target to UnitD failed (UnitD stopped)");
                    }
                }
                _ => unreachable!("unexpected SysD action for StartDefaultTarget"),
            }
        }
    }
}

impl fmt::Debug for Actor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SysDActor")
    }
}

/// 请求 UnitD 停止所有 unit, 并仅在收到响应后回报内部事件.
///
/// 该 task 不执行其它退出逻辑; 后续步骤由状态机在收到 [`M::ExitUnitsDone`] 时决定.
fn spawn_exit_wait(ud: UnitD, self_tx: mpsc::Sender<M>, reply: oneshot::Sender<SdRes>) {
    tokio::spawn(async move {
        let (tx, rx) = oneshot::channel();
        if ud.send(UnitDM::Exit(tx)).await.is_err() {
            error!("send exit to UnitD failed (UnitD stopped)");
        }
        let result = rx
            .await
            .unwrap_or_else(|_| Err(SysdError::Failed("UnitD stopped".to_string())));
        if self_tx.send(M::ExitUnitsDone(result, reply)).await.is_err() {
            error!("send exit-units-done to SysD actor failed (actor stopped)");
        }
    });
}

#[cfg(test)]
mod test;
