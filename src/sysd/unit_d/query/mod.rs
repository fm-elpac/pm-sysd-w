//! 查询类命令的 async task (组合 UnitD 快照与 ServiceD 数据).
//!
//! 状态机 ([`super::sm::Sm`]) 对查询命令只做只读决策, 输出
//! [`QueryReq`] 动作; UnitD 在主循环中 `tokio::spawn` 本模块的 task, 向 ServiceD
//! 查询运行状态, 再结合 UnitD 侧快照合成响应. 响应由 UnitD 发送 (本模块不接触
//! `oneshot::Sender`).

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use tokio::sync::oneshot;

use crate::{
    conf::{Unit, UnitType},
    jsonl::{
        SdRes, SdResPage, SdResServiceStatus, SdResUnitS, SdResUnitState, SdResUnitStatus,
        SdResWhoami, encode_frame,
    },
    sysd::{
        send_reply,
        service_d::{ServiceD, ServiceDM},
        service_s::ServiceStatusInfo,
    },
    sysu::{DaemonSta, EnableSta, LoadSta, ServiceSta, TargetSta, UnitSta, UnitSubSta},
};

/// UnitD 侧的一个 unit 快照 (只读)
#[derive(Debug, Clone, Serialize)]
pub struct UnitInfo {
    pub name: String,
    pub description: String,
    pub unit_type: UnitType,
    pub load: LoadSta,
    pub enable: EnableSta,
}

/// 查询请求 (由状态机输出)
#[derive(Debug, Clone, Serialize)]
pub struct QueryReq {
    pub kind: QueryKind,
}

/// 查询种类
#[derive(Debug, Clone, Serialize)]
pub enum QueryKind {
    /// list-units
    ListUnits {
        /// 当前 target (作为一条 target unit 返回)
        target: String,
        infos: Vec<UnitInfo>,
        /// 每页最多返回条数
        limit: u32,
        /// 分页游标 (`Some(name)` 时读取 `name > cursor`)
        cursor: Option<String>,
        /// 已 mask 的 unit 名称 (list-units 不显示被 mask 的 unit)
        masked: Vec<String>,
    },
    /// unit-state
    UnitState {
        name: String,
        info: Option<UnitInfo>,
    },
    /// status
    Status {
        name: String,
        info: Option<UnitInfo>,
    },
    /// is-system-running
    SystemState {
        /// 当前配置中的 unit 快照 (未 mask)
        infos: Vec<UnitInfo>,
        /// 已 mask 的 unit 名称
        masked: Vec<String>,
    },
    /// whoami (按 pid 查运行中 service)
    Whoami {
        /// 要查询的 pid
        pid: u32,
    },
}

/// 执行查询 (在 async task 中), 返回响应 (由 UnitD 发送)
pub fn spawn_query(req: QueryReq, sd: ServiceD, tx: oneshot::Sender<SdRes>, ipc_m_byte: usize) {
    tokio::spawn(async move {
        let r = match req.kind {
            QueryKind::ListUnits {
                target,
                infos,
                limit,
                cursor,
                masked,
            } => {
                list_units(
                    &target,
                    infos,
                    limit,
                    cursor.as_deref(),
                    &masked,
                    &sd,
                    ipc_m_byte,
                )
                .await
            }
            QueryKind::UnitState { name, info } => unit_state(&name, info, &sd).await,
            QueryKind::Status { name, info } => status(&name, info, &sd).await,
            QueryKind::SystemState { infos, masked } => system_state(&sd, infos, &masked).await,
            QueryKind::Whoami { pid } => whoami(pid, &sd).await,
        };
        send_reply(tx, r);
    });
}

/// 批量查询 active / sub 状态
async fn query_states(
    sd: &ServiceD,
    names: Vec<String>,
) -> Result<Vec<(String, UnitSta, ServiceSta)>, String> {
    let (reply, rx) = oneshot::channel();
    sd.send(ServiceDM::QueryStates { names, reply })
        .await
        .map_err(|_| "ServiceD stopped".to_string())?;
    rx.await.map_err(|_| "ServiceD stopped".to_string())
}

/// 查询 active / sub 状态
async fn query_state(sd: &ServiceD, name: &str) -> Result<(UnitSta, ServiceSta), String> {
    let (reply, rx) = oneshot::channel();
    sd.send(ServiceDM::QueryState {
        name: name.to_string(),
        reply,
    })
    .await
    .map_err(|_| "ServiceD stopped".to_string())?;
    rx.await.map_err(|_| "ServiceD stopped".to_string())
}

/// 查询详细状态
async fn query_status(sd: &ServiceD, name: &str) -> Result<ServiceStatusInfo, String> {
    let (reply, rx) = oneshot::channel();
    sd.send(ServiceDM::QueryStatus {
        name: name.to_string(),
        reply,
    })
    .await
    .map_err(|_| "ServiceD stopped".to_string())?;
    rx.await.map_err(|_| "ServiceD stopped".to_string())
}

/// 查询全部运行中 service 的旧配置 (按名称排序)
async fn query_running_units(sd: &ServiceD) -> Result<Vec<(String, Unit)>, String> {
    let (reply, rx) = oneshot::channel();
    sd.send(ServiceDM::QueryRunningUnits { reply })
        .await
        .map_err(|_| "ServiceD stopped".to_string())?;
    rx.await.map_err(|_| "ServiceD stopped".to_string())
}

/// 查询运行中 service 的旧配置
async fn query_running_unit(sd: &ServiceD, name: &str) -> Result<Option<Unit>, String> {
    let (reply, rx) = oneshot::channel();
    sd.send(ServiceDM::QueryRunningUnit {
        name: name.to_string(),
        reply,
    })
    .await
    .map_err(|_| "ServiceD stopped".to_string())?;
    rx.await.map_err(|_| "ServiceD stopped".to_string())
}

/// list-units: 组合 UnitS 快照、ServiceD 运行状态, 以及从新配置中移除但仍在运行的
/// service 旧配置, 统一按 unit 名称升序排序后分页返回.
async fn list_units(
    target: &str,
    infos: Vec<UnitInfo>,
    limit: u32,
    cursor: Option<&str>,
    masked: &[String],
    sd: &ServiceD,
    ipc_m_byte: usize,
) -> SdRes {
    let masked: HashSet<&str> = masked.iter().map(String::as_str).collect();

    // 当前配置中存在的 unit (不含被 mask 的 unit)
    let mut entries: Vec<SdResUnitState> = infos
        .into_iter()
        .map(|info| SdResUnitState {
            name: info.name,
            description: info.description,
            unit_type: info.unit_type,
            active: UnitSta::Inactive,
            sub: UnitSubSta::Service(ServiceSta::Dead),
            load: info.load,
            enable: info.enable,
        })
        .collect();

    // 已从新配置移除但仍运行的 unit (旧配置).
    // 被 mask 但仍在运行的 unit 也要显示, 并标记为 masked.
    let known: HashSet<String> = entries.iter().map(|e| e.name.clone()).collect();
    let running = match query_running_units(sd).await {
        Ok(v) => v,
        Err(e) => return SdRes::Err(e),
    };
    for (name, unit) in running {
        if known.contains(&name) {
            continue;
        }
        let (load, enable) = if masked.contains(name.as_str()) {
            (LoadSta::Masked, EnableSta::Masked)
        } else {
            (LoadSta::Loaded, EnableSta::Static)
        };
        entries.push(SdResUnitState {
            name: unit.name,
            description: unit.description,
            unit_type: unit.unit_type,
            active: UnitSta::Inactive,
            sub: UnitSubSta::Service(ServiceSta::Dead),
            load,
            enable,
        });
    }

    // 查询这些 service 的运行状态 (当前 target 不属于 ServiceD, 单独追加)
    let names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
    let states = match query_states(sd, names).await {
        Ok(s) => s,
        Err(e) => return SdRes::Err(e),
    };
    let mut state_map: HashMap<String, (UnitSta, ServiceSta)> = states
        .into_iter()
        .map(|(name, active, sub)| (name, (active, sub)))
        .collect();
    for e in &mut entries {
        let (active, sub) = state_map
            .remove(&e.name)
            .unwrap_or((UnitSta::Inactive, ServiceSta::Dead));
        e.active = active;
        e.sub = UnitSubSta::Service(sub);
    }

    // 当前 target 作为一条 target unit
    entries.push(SdResUnitState {
        name: target.to_string(),
        description: String::new(),
        unit_type: UnitType::Target,
        active: UnitSta::Active,
        sub: UnitSubSta::Target(TargetSta::Active),
        load: LoadSta::Loaded,
        enable: EnableSta::Static,
    });

    // 始终按 unit 名称 (字符串默认排序) 排序
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let total = entries.len() as u64;

    // 游标: `Some(name)` 时只返回名称严格大于游标的 unit
    let filtered: Vec<SdResUnitState> = match cursor {
        Some(c) => entries
            .into_iter()
            .filter(|e| e.name.as_str() > c)
            .collect(),
        None => entries,
    };

    // 先按 limit 取一页. 若 JSON 编码后超限, 用 "超限减半" 逐步减少条数.
    // 游标必须与实际编码内容一致, 因此每次都在 encode 前计算.
    let mut units: Vec<SdResUnitState> = filtered.iter().take(limit as usize).cloned().collect();
    loop {
        let next_cursor = if units.len() < filtered.len() {
            units.last().map(|e| e.name.clone())
        } else {
            None
        };
        let resp = SdRes::ListUnits(SdResPage {
            total,
            cursor: next_cursor,
            list: units.clone(),
        });
        let line = match encode_frame(&resp) {
            Ok(line) => line,
            Err(e) => return SdRes::Err(e.to_string()),
        };
        if units.len() <= 1 || line.len() <= ipc_m_byte {
            return resp;
        }
        units.truncate(units.len() / 2);
    }
}

/// unit-state: 组合 UnitS 快照与 ServiceD 运行状态
async fn unit_state(name: &str, info: Option<UnitInfo>, sd: &ServiceD) -> SdRes {
    // 被 mask 的 unit 也要查询 ServiceD, 因为 service 可能仍在运行 (mask 未 --now)
    if info.as_ref().is_some_and(|i| i.load == LoadSta::Masked) {
        return match query_running_unit(sd, name).await {
            Ok(Some(c)) => {
                let (active, sub) = match query_state(sd, name).await {
                    Ok(s) => s,
                    Err(e) => return SdRes::Err(e),
                };
                SdRes::UnitState(Some(SdResUnitState {
                    name: c.name,
                    description: c.description,
                    unit_type: c.unit_type,
                    active,
                    sub: UnitSubSta::Service(sub),
                    load: LoadSta::Masked,
                    enable: EnableSta::Masked,
                }))
            }
            Ok(None) => {
                let i = info.expect("masked info");
                SdRes::UnitState(Some(SdResUnitState {
                    name: i.name,
                    description: i.description,
                    unit_type: i.unit_type,
                    active: UnitSta::Inactive,
                    sub: UnitSubSta::Service(ServiceSta::Dead),
                    load: LoadSta::Masked,
                    enable: EnableSta::Masked,
                }))
            }
            Err(e) => SdRes::Err(e),
        };
    }

    // 配置优先读 UnitS
    // 若已被 daemon-reload 移除但仍 运行中, 从 ServiceD 取旧配置
    let (cname, cdesc, ctype, load, enable) = match info {
        Some(i) => (i.name, i.description, i.unit_type, i.load, i.enable),
        None => match query_running_unit(sd, name).await {
            Ok(Some(c)) => (
                c.name,
                c.description,
                c.unit_type,
                LoadSta::Loaded,
                EnableSta::Static,
            ),
            Ok(None) => return SdRes::UnitState(None),
            Err(e) => return SdRes::Err(e),
        },
    };

    let (active, sub) = match query_state(sd, name).await {
        Ok(s) => s,
        Err(e) => return SdRes::Err(e),
    };

    SdRes::UnitState(Some(SdResUnitState {
        name: cname,
        description: cdesc,
        unit_type: ctype,
        active,
        sub: UnitSubSta::Service(sub),
        load,
        enable,
    }))
}

/// status: 组合 UnitS 快照与 ServiceD 详细状态
async fn status(name: &str, info: Option<UnitInfo>, sd: &ServiceD) -> SdRes {
    // 被 mask 的 unit 也要查询 ServiceD, 因为 service 可能仍在运行 (mask 未 --now).
    if info.as_ref().is_some_and(|i| i.load == LoadSta::Masked) {
        return match query_running_unit(sd, name).await {
            Ok(Some(c)) => {
                let st = match query_status(sd, name).await {
                    Ok(s) => s,
                    Err(e) => return SdRes::Err(e),
                };
                SdRes::Status(SdResUnitStatus {
                    name: c.name,
                    description: c.description,
                    load: LoadSta::Masked,
                    enable: EnableSta::Masked,
                    active: st.active,
                    sub: UnitSubSta::Service(st.sub),
                    since: st.since,
                    s: SdResUnitS::Service(SdResServiceStatus {
                        pid: st.pid,
                        restart_count: st.restart_count,
                        exit_status: st.exit_status,
                    }),
                })
            }
            Ok(None) => {
                let i = info.expect("masked info");
                SdRes::Status(SdResUnitStatus {
                    name: i.name,
                    description: i.description,
                    load: LoadSta::Masked,
                    enable: EnableSta::Masked,
                    active: UnitSta::Inactive,
                    sub: UnitSubSta::Service(ServiceSta::Dead),
                    since: None,
                    s: SdResUnitS::Service(SdResServiceStatus {
                        pid: None,
                        restart_count: 0,
                        exit_status: None,
                    }),
                })
            }
            Err(e) => SdRes::Err(e),
        };
    }

    let (cname, cdesc, load, enable) = match info {
        Some(i) => (i.name, i.description, i.load, i.enable),
        None => match query_running_unit(sd, name).await {
            Ok(Some(c)) => (c.name, c.description, LoadSta::Loaded, EnableSta::Static),
            Ok(None) => return SdRes::Err(format!("Unit {} not found.", name)),
            Err(e) => return SdRes::Err(e),
        },
    };

    let st = match query_status(sd, name).await {
        Ok(s) => s,
        Err(e) => return SdRes::Err(e),
    };

    SdRes::Status(SdResUnitStatus {
        name: cname,
        description: cdesc,
        load,
        enable,
        active: st.active,
        sub: UnitSubSta::Service(st.sub),
        since: st.since,
        s: SdResUnitS::Service(SdResServiceStatus {
            pid: st.pid,
            restart_count: st.restart_count,
            exit_status: st.exit_status,
        }),
    })
}

/// whoami: 按 pid 反查运行中 service 名称.
async fn whoami(pid: u32, sd: &ServiceD) -> SdRes {
    let (reply, rx) = oneshot::channel();
    if sd.send(ServiceDM::QueryPid { pid, reply }).await.is_err() {
        return SdRes::Err("ServiceD stopped".to_string());
    }
    match rx.await {
        Ok(Some(name)) => SdRes::Whoami(SdResWhoami {
            service: Some(name),
            init: None,
        }),
        Ok(None) => SdRes::Err(format!(
            "PID {} does not belong to any running service.",
            pid
        )),
        Err(_) => SdRes::Err("ServiceD stopped".to_string()),
    }
}

/// is-system-running: 组合当前配置与 ServiceD 运行状态, 计算整个实例的运行状态.
///
/// 本项目没有 basic.target / job 队列 / rescue target, 因此只区分:
/// `running` (正常) / `degraded` (存在 failed unit) / `unknown` (查询失败).
/// `stopping` 由 UnitD 在进入退出状态时直接回复, 不在本查询中计算.
async fn system_state(sd: &ServiceD, infos: Vec<UnitInfo>, masked: &[String]) -> SdRes {
    // 查询范围: 当前配置中的 unit, 已 mask 但仍可能运行的 unit, 以及已从配置
    // 移除但仍运行的旧配置 service.
    let mut names: Vec<String> = infos.iter().map(|i| i.name.clone()).collect();
    names.extend(masked.iter().cloned());

    let running = match query_running_units(sd).await {
        Ok(v) => v,
        Err(_) => return SdRes::SystemState(DaemonSta::Unknown),
    };
    for (name, _) in running {
        if !names.contains(&name) {
            names.push(name);
        }
    }

    let states = match query_states(sd, names).await {
        Ok(s) => s,
        Err(_) => return SdRes::SystemState(DaemonSta::Unknown),
    };
    let degraded = states
        .iter()
        .any(|(_, active, _)| *active == UnitSta::Failed);
    SdRes::SystemState(if degraded {
        DaemonSta::Degraded
    } else {
        DaemonSta::Running
    })
}
