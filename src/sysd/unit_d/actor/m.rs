//! UnitD actor 消息类型.

use std::fmt;

use serde::{Serialize, Serializer, ser::SerializeTupleVariant};
use tokio::sync::oneshot;

use crate::{
    jsonl::{SdReq, SdRes},
    sysd::{SysdError, reload::ReloadR, unit_d::sm::PendingWrite},
};

/// 发给 UnitD 的外部消息 (pub API).
///
/// 每条消息自带 oneshot 回执, 由调用方构造.
pub enum UnitDM {
    /// 处理一条 ctl 请求 (直接复用 jsonl 类型), 回执为 `SdRes`
    SdReq(SdReq, oneshot::Sender<SdRes>),
    /// 启动默认 target, 回执为命令结果
    StartDefaultTarget(oneshot::Sender<Result<(), SysdError>>),
    /// 停止所有 unit, 然后退出本 actor 主循环
    Exit(oneshot::Sender<Result<(), SysdError>>),
}

/// 发给 actor 的内部消息 (外部消息与内部事件共用同一通道)
pub enum M {
    /// 外部消息包装
    M(UnitDM),
    /// daemon-reload 加载完成 (携带完整加载结果与原始回执)
    ReloadOk(ReloadR, oneshot::Sender<SdRes>),
    /// daemon-reload 加载失败 (携带错误消息与原始回执)
    ReloadErr(String, oneshot::Sender<SdRes>),
    /// 数据库写入完成 (携带结果, 对应的后续操作, 与原始回执)
    WriteDbDone(Result<(), SysdError>, PendingWrite, oneshot::Sender<SdRes>),
    /// ServiceD 已停止所有 service (携带结果与原始回执)
    ExitServiceDone(
        Result<(), SysdError>,
        oneshot::Sender<Result<(), SysdError>>,
    ),
}

impl fmt::Debug for UnitDM {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SdReq(req, _) => f.debug_tuple("SdReq").field(req).finish(),
            Self::StartDefaultTarget(_) => f.write_str("StartDefaultTarget"),
            Self::Exit(_) => f.write_str("Exit"),
        }
    }
}

impl fmt::Debug for M {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::M(m) => f.debug_tuple("M").field(m).finish(),
            Self::ReloadOk(load, _) => f.debug_tuple("ReloadOk").field(load).finish(),
            Self::ReloadErr(e, _) => f.debug_tuple("ReloadErr").field(e).finish(),
            Self::WriteDbDone(result, pending, _) => f
                .debug_tuple("WriteDbDone")
                .field(result)
                .field(pending)
                .finish(),
            Self::ExitServiceDone(result, _) => {
                f.debug_tuple("ExitServiceDone").field(result).finish()
            }
        }
    }
}

impl Serialize for UnitDM {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::SdReq(req, _) => serializer.serialize_newtype_variant("UnitDM", 0, "SdReq", req),
            Self::StartDefaultTarget(_) => {
                serializer.serialize_unit_variant("UnitDM", 1, "StartDefaultTarget")
            }
            Self::Exit(_) => serializer.serialize_unit_variant("UnitDM", 2, "Exit"),
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
            Self::ReloadOk(r, _) => {
                let mut v = serializer.serialize_tuple_variant("M", 1, "ReloadOk", 1)?;
                v.serialize_field(r)?;
                v.end()
            }
            Self::ReloadErr(e, _) => serializer.serialize_newtype_variant("M", 2, "ReloadErr", e),
            Self::WriteDbDone(result, pending, _) => {
                let mut v = serializer.serialize_tuple_variant("M", 3, "WriteDbDone", 2)?;
                v.serialize_field(result)?;
                v.serialize_field(pending)?;
                v.end()
            }
            Self::ExitServiceDone(result, _) => {
                serializer.serialize_newtype_variant("M", 4, "ExitServiceDone", result)
            }
        }
    }
}
