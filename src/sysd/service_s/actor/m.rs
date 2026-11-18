//! ServiceS actor 消息类型.

use std::fmt;

use serde::{Serialize, Serializer, ser::SerializeStructVariant};
use tokio::sync::oneshot;

use crate::{
    sysd::{SysdError, service_d::ServiceOp},
    sysu::{ServiceSta, UnitSta},
};

use super::super::sm::{ExitInfo, ServiceStatusInfo};

/// 命令回执
pub type CmdReply = oneshot::Sender<Result<(), SysdError>>;

/// 发给 ServiceS 的外部消息 (pub API).
///
/// 每条消息自带 oneshot 回执, 由调用方构造; ServiceS 处理完后通过该回执回复.
pub enum ServiceSM {
    /// 单个操作 (Start / Stop / ...), 回执为命令结果
    Cmd(ServiceOp, CmdReply),
    /// 查询 active / sub 状态
    QueryState(oneshot::Sender<(UnitSta, ServiceSta)>),
    /// 查询详细状态
    QueryStatus(oneshot::Sender<ServiceStatusInfo>),
    /// 退出 ServiceS actor 主循环 (由 ServiceD 在收到退出请求后发送)
    Exit,
}

/// 发给 actor 的内部消息 (外部消息与内部事件共用同一通道)
pub enum M {
    /// 外部消息包装
    M(ServiceSM),
    /// 子进程已创建
    Spawned { pid: Option<u32> },
    /// 子进程创建失败
    SpawnFailed(SysdError),
    /// 子进程已退出
    ChildExited(ExitInfo),
    /// ExecStop 执行完成
    ExecStopDone,
    /// ExecReload 执行完成 (携带结果)
    ReloadDone(Result<(), SysdError>),
    /// 重启定时器到期
    RestartTimer,
    /// 启动超时定时器到期
    StartTimeout,
}

impl fmt::Debug for ServiceSM {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cmd(op, _) => f.debug_tuple("Cmd").field(op).finish(),
            Self::QueryState(_) => f.write_str("QueryState"),
            Self::QueryStatus(_) => f.write_str("QueryStatus"),
            Self::Exit => f.write_str("Exit"),
        }
    }
}

impl fmt::Debug for M {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::M(m) => f.debug_tuple("M").field(m).finish(),
            Self::Spawned { pid } => f.debug_struct("Spawned").field("pid", pid).finish(),
            Self::SpawnFailed(e) => f.debug_tuple("SpawnFailed").field(e).finish(),
            Self::ChildExited(info) => f.debug_tuple("ChildExited").field(info).finish(),
            Self::ExecStopDone => f.write_str("ExecStopDone"),
            Self::ReloadDone(result) => f.debug_tuple("ReloadDone").field(result).finish(),
            Self::RestartTimer => f.write_str("RestartTimer"),
            Self::StartTimeout => f.write_str("StartTimeout"),
        }
    }
}

impl Serialize for ServiceSM {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Cmd(op, _) => serializer.serialize_newtype_variant("ServiceSM", 0, "Cmd", op),
            Self::QueryState(_) => serializer.serialize_unit_variant("ServiceSM", 1, "QueryState"),
            Self::QueryStatus(_) => {
                serializer.serialize_unit_variant("ServiceSM", 2, "QueryStatus")
            }
            Self::Exit => serializer.serialize_unit_variant("ServiceSM", 3, "Exit"),
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
            Self::Spawned { pid } => {
                let mut v = serializer.serialize_struct_variant("M", 1, "Spawned", 1)?;
                v.serialize_field("pid", pid)?;
                v.end()
            }
            Self::SpawnFailed(e) => serializer.serialize_newtype_variant("M", 2, "SpawnFailed", e),
            Self::ChildExited(info) => {
                serializer.serialize_newtype_variant("M", 3, "ChildExited", info)
            }
            Self::ExecStopDone => serializer.serialize_unit_variant("M", 4, "ExecStopDone"),
            Self::ReloadDone(result) => {
                serializer.serialize_newtype_variant("M", 5, "ReloadDone", result)
            }
            Self::RestartTimer => serializer.serialize_unit_variant("M", 6, "RestartTimer"),
            Self::StartTimeout => serializer.serialize_unit_variant("M", 7, "StartTimeout"),
        }
    }
}
