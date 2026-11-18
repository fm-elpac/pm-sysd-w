//! ServiceD actor 消息类型.

use std::fmt;

use serde::{Serialize, Serializer, ser::SerializeStructVariant};
use tokio::sync::oneshot;

use crate::{
    conf::Unit,
    jsonl::SdRes,
    sysd::{
        SysdError,
        service_d::ServiceCmdUnit,
        service_s::{ServiceStatus, ServiceStatusInfo},
    },
    sysu::{ServiceSta, UnitSta},
};

/// 发给 ServiceD 的外部消息 (pub API).
///
/// 每条消息自带 oneshot 回执, 由调用方构造.
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
        reply: oneshot::Sender<ServiceStatusInfo>,
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
    /// 读取全部运行中 service 的旧配置 (供 list-units 补齐)
    QueryRunningUnits {
        reply: oneshot::Sender<Vec<(String, Unit)>>,
    },
}

/// ServiceS 发给 ServiceD 的消息 (通过 [`super::ServiceD::send_s`] 投递)
pub enum MS {
    /// ServiceS 请求退出, 并上报终止状态. ServiceD 收到后回发
    /// [`crate::sysd::service_s::ServiceSM::Exit`].
    ExitRequest { name: String, status: ServiceStatus },
}

/// 发给 actor 的内部消息 (外部消息与 ServiceS 回报共用同一通道)
pub enum M {
    /// 外部消息包装
    M(ServiceDM),
    /// ServiceS 回报包装
    S(MS),
}

impl fmt::Debug for ServiceDM {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unit(cmd, _) => f.debug_tuple("Unit").field(cmd).finish(),
            Self::StartMany(services, _) => f.debug_tuple("StartMany").field(services).finish(),
            Self::Exit(_) => f.write_str("Exit"),
            Self::QueryState { name, .. } => {
                f.debug_struct("QueryState").field("name", name).finish()
            }
            Self::QueryStatus { name, .. } => {
                f.debug_struct("QueryStatus").field("name", name).finish()
            }
            Self::QueryStates { names, .. } => {
                f.debug_struct("QueryStates").field("names", names).finish()
            }
            Self::QueryRunningUnit { name, .. } => f
                .debug_struct("QueryRunningUnit")
                .field("name", name)
                .finish(),
            Self::QueryRunningUnits { .. } => f.write_str("QueryRunningUnits"),
        }
    }
}

impl fmt::Debug for MS {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExitRequest { name, status } => f
                .debug_struct("ExitRequest")
                .field("name", name)
                .field("status", status)
                .finish(),
        }
    }
}

impl fmt::Debug for M {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::M(m) => f.debug_tuple("M").field(m).finish(),
            Self::S(m) => f.debug_tuple("S").field(m).finish(),
        }
    }
}

impl Serialize for ServiceDM {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Unit(cmd, _) => serializer.serialize_newtype_variant("ServiceDM", 0, "Unit", cmd),
            Self::StartMany(services, _) => {
                serializer.serialize_newtype_variant("ServiceDM", 1, "StartMany", services)
            }
            Self::Exit(_) => serializer.serialize_unit_variant("ServiceDM", 2, "Exit"),
            Self::QueryState { name, .. } => {
                let mut v = serializer.serialize_struct_variant("ServiceDM", 3, "QueryState", 1)?;
                v.serialize_field("name", name)?;
                v.end()
            }
            Self::QueryStatus { name, .. } => {
                let mut v =
                    serializer.serialize_struct_variant("ServiceDM", 4, "QueryStatus", 1)?;
                v.serialize_field("name", name)?;
                v.end()
            }
            Self::QueryStates { names, .. } => {
                let mut v =
                    serializer.serialize_struct_variant("ServiceDM", 5, "QueryStates", 1)?;
                v.serialize_field("names", names)?;
                v.end()
            }
            Self::QueryRunningUnit { name, .. } => {
                let mut v =
                    serializer.serialize_struct_variant("ServiceDM", 6, "QueryRunningUnit", 1)?;
                v.serialize_field("name", name)?;
                v.end()
            }
            Self::QueryRunningUnits { .. } => {
                serializer.serialize_unit_variant("ServiceDM", 7, "QueryRunningUnits")
            }
        }
    }
}

impl Serialize for MS {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::ExitRequest { name, status } => {
                let mut v = serializer.serialize_struct_variant("MS", 0, "ExitRequest", 2)?;
                v.serialize_field("name", name)?;
                v.serialize_field("status", status)?;
                v.end()
            }
        }
    }
}

impl Serialize for M {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::M(m) => serializer.serialize_newtype_variant("M", 0, "M", m),
            Self::S(m) => serializer.serialize_newtype_variant("M", 1, "S", m),
        }
    }
}
