//! 查询类命令的 async task (组合活跃 ServiceS 与最近终止状态).
//!
//! ServiceD 主循环对查询命令不做决策: 直接读取活跃 ServiceS 句柄 (主循环持有)
//! 与最近终止状态 (状态机持有), 交给本模块回复. 活跃 service 查询需要在 async task
//! 中等待 ServiceS 回复; 非活跃 service 直接由终止状态推导.

use tokio::sync::oneshot;

use crate::{
    sysd::{
        send_reply,
        service_s::{ServiceS, ServiceSM, ServiceStatus, ServiceStatusInfo},
    },
    sysu::{ServiceSta, UnitSta},
};

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
                send_reply(reply, state);
            });
        }
        None => {
            send_reply(reply, active_sub_of(last.as_ref()));
        }
    }
}

/// 查询详细状态
pub fn spawn_query_status(
    handle: Option<ServiceS>,
    last: Option<ServiceStatus>,
    reply: oneshot::Sender<ServiceStatusInfo>,
) {
    match handle {
        Some(h) => {
            tokio::spawn(async move {
                let (tx, rx) = oneshot::channel();
                let info = if h.send(ServiceSM::QueryStatus(tx)).await.is_err() {
                    ServiceStatusInfo::inactive()
                } else {
                    rx.await.unwrap_or_else(|_| ServiceStatusInfo::inactive())
                };
                send_reply(reply, info);
            });
        }
        None => {
            send_reply(reply, status_info_of(last.as_ref()));
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
        send_reply(reply, out);
    });
}

/// 按 pid 反查运行中 service 名称.
///
/// 遍历所有活跃 ServiceS 句柄并查询其详细状态, 找到 `pid` 匹配的 service 后
/// 立即回复其名称; 全部未命中则回复 `None`.
pub fn spawn_query_pid(
    handles: Vec<(String, ServiceS)>,
    pid: u32,
    reply: oneshot::Sender<Option<String>>,
) {
    tokio::spawn(async move {
        for (name, h) in handles {
            let (tx, rx) = oneshot::channel();
            // 单个 service 查询失败视为未命中 (与其它查询命令相同的尽力而为语义).
            if h.send(ServiceSM::QueryStatus(tx)).await.is_err() {
                continue;
            }
            let info = match rx.await {
                Ok(info) => info,
                Err(_) => continue,
            };
            if info.pid == Some(pid) {
                send_reply(reply, Some(name));
                return;
            }
        }
        send_reply(reply, None);
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
pub fn status_info_of(last: Option<&ServiceStatus>) -> ServiceStatusInfo {
    match last {
        Some(s) => {
            let (active, sub) = s.active_sub();
            ServiceStatusInfo {
                active,
                sub,
                pid: None,
                restart_count: s.restart_count,
                exit_status: s.exit_status,
                since: s.since,
            }
        }
        None => ServiceStatusInfo::inactive(),
    }
}
