//! SysD actor 消息类型.

use std::fmt;

use serde::{Serialize, Serializer};
use tokio::sync::oneshot;

use crate::{
    jsonl::{SdReq, SdRes},
    sysd::SysdError,
};

/// 发给 SysD 的外部消息 (pub API).
///
/// 每条消息自带 oneshot 回执, 由调用方构造.
pub enum SysDM {
    /// 处理一条 ctl 请求 (直接复用 jsonl 类型), 回执为 `SdRes`
    SdReq(SdReq, oneshot::Sender<SdRes>),
    /// 启动默认 target, 回执为命令结果
    StartDefaultTarget(oneshot::Sender<Result<(), SysdError>>),
}

/// 发给 actor 的内部消息 (外部消息与内部事件共用同一通道)
pub enum M {
    /// 外部消息包装
    M(SysDM),
    /// UnitD 已启动 default.target (携带结果与原始回执)
    StartDefaultTargetDone(
        Result<(), SysdError>,
        oneshot::Sender<Result<(), SysdError>>,
    ),
    /// UnitD 已停止所有 unit (退出序列)
    ExitUnitsDone(Result<(), SysdError>, oneshot::Sender<SdRes>),
}

impl fmt::Debug for SysDM {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SdReq(req, _) => f.debug_tuple("SdReq").field(req).finish(),
            Self::StartDefaultTarget(_) => f.write_str("StartDefaultTarget"),
        }
    }
}

impl fmt::Debug for M {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::M(m) => f.debug_tuple("M").field(m).finish(),
            Self::StartDefaultTargetDone(result, _) => f
                .debug_tuple("StartDefaultTargetDone")
                .field(result)
                .finish(),
            Self::ExitUnitsDone(result, _) => f.debug_tuple("ExitUnitsDone").field(result).finish(),
        }
    }
}

impl Serialize for SysDM {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::SdReq(req, _) => serializer.serialize_newtype_variant("SysDM", 0, "SdReq", req),
            Self::StartDefaultTarget(_) => {
                serializer.serialize_unit_variant("SysDM", 1, "StartDefaultTarget")
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
            Self::StartDefaultTargetDone(result, _) => {
                serializer.serialize_newtype_variant("M", 1, "StartDefaultTargetDone", result)
            }
            Self::ExitUnitsDone(result, _) => {
                serializer.serialize_newtype_variant("M", 2, "ExitUnitsDone", result)
            }
        }
    }
}
