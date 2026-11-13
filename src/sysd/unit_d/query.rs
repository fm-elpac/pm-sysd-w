//! 查询类命令的 async task (组合 UnitD 快照与 ServiceD 数据).
//!
//! 状态机 ([`super::actor::UnitDSM`]) 对查询命令只做只读决策, 输出
//! [`QueryReq`] 动作; UnitD 在主循环中 `tokio::spawn` 本模块的 task, 向 ServiceD
//! 查询运行状态, 再结合 UnitD 侧快照合成响应. 响应由 UnitD 发送 (本模块不接触
//! `oneshot::Sender`).

use std::collections::HashMap;

use tokio::sync::oneshot;

use crate::{
    conf::{Unit, UnitType},
    jsonl::{SdRes, SdResServiceStatus, SdResUnitS, SdResUnitState, SdResUnitStatus},
    sysd::{
        ServiceSta, TargetSta, UnitSta, UnitSubSta,
        service_d::{ServiceD, ServiceDM, StatusInfo},
    },
};

/// UnitD 侧的一个 unit 快照 (只读)
#[derive(Debug, Clone)]
pub struct UnitInfo {
    pub name: String,
    pub description: String,
    pub unit_type: UnitType,
    pub enabled: bool,
    pub masked: bool,
}

/// 查询请求 (由状态机输出)
pub struct QueryReq {
    pub kind: QueryKind,
}

/// 查询种类
pub enum QueryKind {
    /// list-units
    ListUnits {
        /// 当前 target (作为一条 target unit 返回)
        target: String,
        infos: Vec<UnitInfo>,
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
}

/// 执行查询 (在 async task 中), 返回响应 (由 UnitD 发送)
pub fn spawn_query(req: QueryReq, sd: ServiceD, tx: oneshot::Sender<SdRes>) {
    tokio::spawn(async move {
        let r = match req.kind {
            QueryKind::ListUnits { target, infos } => list_units(&target, infos, &sd).await,
            QueryKind::UnitState { name, info } => unit_state(&name, info, &sd).await,
            QueryKind::Status { name, info } => status(&name, info, &sd).await,
        };
        tx.send(r).expect("unitd reply channel closed");
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
async fn query_status(sd: &ServiceD, name: &str) -> Result<StatusInfo, String> {
    let (reply, rx) = oneshot::channel();
    sd.send(ServiceDM::QueryStatus {
        name: name.to_string(),
        reply,
    })
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

/// list-units: 组合 UnitS 快照与 ServiceD 运行状态, 并在末尾追加当前 target 条目
async fn list_units(target: &str, infos: Vec<UnitInfo>, sd: &ServiceD) -> SdRes {
    let names: Vec<String> = infos.iter().map(|i| i.name.clone()).collect();
    let states = match query_states(sd, names).await {
        Ok(s) => s,
        Err(e) => return SdRes::Err(e),
    };
    let mut state_map: HashMap<String, (UnitSta, ServiceSta)> = states
        .into_iter()
        .map(|(name, active, sub)| (name, (active, sub)))
        .collect();

    let mut out: Vec<SdResUnitState> = infos
        .into_iter()
        .map(|info| {
            let (active, sub) = state_map
                .remove(&info.name)
                .unwrap_or((UnitSta::Inactive, ServiceSta::Dead));
            SdResUnitState {
                name: info.name,
                description: info.description,
                unit_type: info.unit_type,
                active,
                sub: UnitSubSta::Service(sub),
                enabled: info.enabled,
                masked: info.masked,
            }
        })
        .collect();

    // 当前 target 作为 一条 target unit 追加到列表末尾.
    out.push(SdResUnitState {
        name: target.to_string(),
        description: String::new(),
        unit_type: UnitType::Target,
        active: UnitSta::Active,
        sub: UnitSubSta::Target(TargetSta::Active),
        enabled: false,
        masked: false,
    });

    SdRes::ListUnits(out)
}

/// unit-state: 组合 UnitS 快照与 ServiceD 运行状态
async fn unit_state(name: &str, info: Option<UnitInfo>, sd: &ServiceD) -> SdRes {
    // 配置优先读 UnitS
    // 若已被 daemon-reload 移除但仍 运行中, 从 ServiceD 取旧配置
    let (cname, cdesc, ctype, enabled, masked) = match info {
        Some(i) => (i.name, i.description, i.unit_type, i.enabled, i.masked),
        None => match query_running_unit(sd, name).await {
            Ok(Some(c)) => (c.name, c.description, c.unit_type, false, false),
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
        enabled,
        masked,
    }))
}

/// status: 组合 UnitS 快照与 ServiceD 详细状态
async fn status(name: &str, info: Option<UnitInfo>, sd: &ServiceD) -> SdRes {
    let (cname, cdesc, enabled, masked) = match info {
        Some(i) => (i.name, i.description, i.enabled, i.masked),
        None => match query_running_unit(sd, name).await {
            Ok(Some(c)) => (c.name, c.description, false, false),
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
        enabled,
        masked,
        active: st.active,
        s: SdResUnitS::Service(SdResServiceStatus {
            sub: st.sub,
            pid: st.pid,
            restart_count: st.restart_count,
            exit_status: st.exit_status,
        }),
    })
}
