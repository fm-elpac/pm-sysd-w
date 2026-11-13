//! SysD: 整个 pm-sysd-init 管理实例 (顶级实体).
//!
//! SysD 是 `tokio::spawn` 的 async task. 只负责路由:
//!
//! - 自己处理 `SdReq::SysD` (退出 / soft-reboot).
//! - 日志查询 `SdReq::LogRO` 转发给 Logd; 其余 ctl 请求与 `StartDefaultTarget`
//!   原样转发给 UnitD.
//!
//! 转发不阻塞 SysD 主循环: 每条消息自带 oneshot 回执, 转发给 Logd / UnitD 时
//! 直接透传 (仅等待消息投递完成, 不等待处理/回执). 只有处理 `SdReq::SysD` 时,
//! SysD 才会等待 UnitD 停止所有 unit.
//!
//! 退出时: 先通知 UnitD 停止所有 unit, 再通过 `exit_tx` 通知 `sysd::init`,
//! 最后退出自己的循环.

use tokio::sync::{mpsc, oneshot};

use crate::{
    conf::Unit,
    db::DbClient,
    jsonl::{SdReq, SdRes},
    logd::Logd,
};

use super::{
    SysdError,
    init::Shutdown,
    unit_d::{UnitD, UnitDM},
};

mod actor;

use actor::SysDSM;

/// 发给 SysD 的消息.
///
/// 每条消息自带 oneshot 回执, 由调用方构造; 转发给 UnitD 时直接透传.
pub enum SysDM {
    /// 处理一条 ctl 请求 (直接复用 jsonl 类型), 回执为 `SdRes`.
    SdReq((SdReq, oneshot::Sender<SdRes>)),
    /// 启动默认 target, 回执为命令结果.
    StartDefaultTarget(oneshot::Sender<Result<(), SysdError>>),
}

/// SysD 句柄 (整个 pm-sysd-init 管理实例)
#[derive(Clone)]
pub struct SysD {
    tx: mpsc::Sender<SysDM>,
}

impl SysD {
    /// 启动 SysD (并启动其依赖的 UnitD), 返回句柄.
    ///
    /// `exit_tx` 用于 SysD 决定退出时通知 `sysd::init`.
    pub fn spawn(
        db: DbClient,
        logd: Logd,
        conf_path: &str,
        units: Vec<Unit>,
        exit_tx: mpsc::Sender<Shutdown>,
    ) -> Self {
        let ud = UnitD::spawn(db, logd.clone(), conf_path.to_string(), units);

        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move { run(rx, ud, logd, exit_tx, SysDSM).await });

        Self { tx }
    }

    /// 发送一条消息 (等待投递完成, 不等待处理/回执).
    pub async fn send(&self, msg: SysDM) -> Result<(), mpsc::error::SendError<SysDM>> {
        self.tx.send(msg).await
    }
}

/// SysD 主循环
async fn run(
    mut rx: mpsc::Receiver<SysDM>,
    ud: UnitD,
    logd: Logd,
    exit_tx: mpsc::Sender<Shutdown>,
    sm: SysDSM,
) {
    while let Some(msg) = rx.recv().await {
        match msg {
            // 自己处理: 退出 / soft-reboot. 这里等待 UnitD 停止所有 unit.
            SysDM::SdReq((SdReq::SysD(req), reply)) => {
                let action = sm.handle(&req);

                logd.write_with(action.log_level, None, action.log_msg)
                    .unwrap();

                // (1) 通知 UnitD 停止所有 unit, 并等待完成.
                let (tx, rx) = oneshot::channel();
                ud.send(UnitDM::Exit(tx)).await.unwrap();
                rx.await.unwrap().unwrap();

                // (2) 通知 sysd::init 退出
                exit_tx.send(action.shutdown).await.unwrap();

                // (3) 回复后退出自己的循环
                reply.send(SdRes::Ok).unwrap();
                break;
            }
            // 日志查询: 直接转发给 Logd (不等待处理/回执).
            SysDM::SdReq((SdReq::LogRO(req), reply)) => {
                logd.req_log_ro(req, reply).await.unwrap();
            }
            // 其余 ctl 请求: 直接转发给 UnitD (不等待处理/回执).
            SysDM::SdReq((req, reply)) => {
                ud.send(UnitDM::SdReq(req, reply)).await.unwrap();
            }
            // 启动默认 target: 直接转发给 UnitD.
            SysDM::StartDefaultTarget(reply) => {
                ud.send(UnitDM::StartDefaultTarget(reply)).await.unwrap();
            }
        }
    }
}
