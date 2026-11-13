//! 查询类命令的 async task (组合活跃 ServiceS 与最近终止状态).
//!
//! ServiceD 主循环对查询命令不做决策: 直接读取活跃 ServiceS 句柄 (主循环持有)
//! 与最近终止状态 (状态机持有), 交给本模块回复. 活跃 service 查询需要在 async task
//! 中等待 ServiceS 回复; 非活跃 service 直接由终止状态推导.

use tokio::sync::oneshot;

use crate::sysd::{ServiceSta, UnitSta};

use super::super::service_s::{ServiceS, ServiceSM, ServiceStatus};
use super::StatusInfo;

/// 查询 active / sub 状态
pub fn spawn_query_state(
    handle: Option<ServiceS>,
    last: Option<ServiceStatus>,
    reply: oneshot::Sender<(UnitSta, ServiceSta)>,
) {
    match handle {
        Some(h) => {
            tokio::spawn(async move {
                let (tx, rx) = oneshot::channel();
                let state = if h.send(ServiceSM::QueryState(tx)).await.is_err() {
                    (UnitSta::Inactive, ServiceSta::Dead)
                } else {
                    rx.await.unwrap_or((UnitSta::Inactive, ServiceSta::Dead))
                };
                reply.send(state).expect("query-state reply channel closed");
            });
        }
        None => {
            reply
                .send(active_sub_of(last.as_ref()))
                .expect("query-state reply channel closed");
        }
    }
}

/// 查询详细状态
pub fn spawn_query_status(
    handle: Option<ServiceS>,
    last: Option<ServiceStatus>,
    reply: oneshot::Sender<StatusInfo>,
) {
    match handle {
        Some(h) => {
            tokio::spawn(async move {
                let (tx, rx) = oneshot::channel();
                let (active, sub, pid, restart_count, exit_status) =
                    if h.send(ServiceSM::QueryStatus(tx)).await.is_err() {
                        (UnitSta::Inactive, ServiceSta::Dead, None, 0, None)
                    } else {
                        rx.await
                            .unwrap_or((UnitSta::Inactive, ServiceSta::Dead, None, 0, None))
                    };
                reply
                    .send(StatusInfo {
                        active,
                        sub,
                        pid,
                        restart_count,
                        exit_status,
                    })
                    .expect("query-status reply channel closed");
            });
        }
        None => {
            let (active, sub, pid, restart_count, exit_status) = status_info_of(last.as_ref());
            reply
                .send(StatusInfo {
                    active,
                    sub,
                    pid,
                    restart_count,
                    exit_status,
                })
                .expect("query-status reply channel closed");
        }
    }
}

/// 批量查询 active / sub 状态
pub fn spawn_query_states(
    items: Vec<(String, Option<ServiceS>, Option<ServiceStatus>)>,
    reply: oneshot::Sender<Vec<(String, UnitSta, ServiceSta)>>,
) {
    tokio::spawn(async move {
        let mut out = Vec::with_capacity(items.len());
        for (name, handle, last) in items {
            let (active, sub) = match handle {
                Some(h) => {
                    let (tx, rx) = oneshot::channel();
                    if h.send(ServiceSM::QueryState(tx)).await.is_err() {
                        (UnitSta::Inactive, ServiceSta::Dead)
                    } else {
                        rx.await.unwrap_or((UnitSta::Inactive, ServiceSta::Dead))
                    }
                }
                None => active_sub_of(last.as_ref()),
            };
            out.push((name, active, sub));
        }
        reply.send(out).expect("query-states reply channel closed");
    });
}

/// 由终止状态推导 active / sub 状态 (无活跃 ServiceS 时)
pub fn active_sub_of(last: Option<&ServiceStatus>) -> (UnitSta, ServiceSta) {
    match last {
        Some(s) => s.active_sub(),
        None => (UnitSta::Inactive, ServiceSta::Dead),
    }
}

/// 由终止状态推导详细状态
pub fn status_info_of(
    last: Option<&ServiceStatus>,
) -> (UnitSta, ServiceSta, Option<u32>, u32, Option<i32>) {
    match last {
        Some(s) => {
            let (active, sub) = s.active_sub();
            (active, sub, None, s.restart_count, s.exit_status)
        }
        None => (UnitSta::Inactive, ServiceSta::Dead, None, 0, None),
    }
}
