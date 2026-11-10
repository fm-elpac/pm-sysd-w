//! 错误处理.
//!
//! ## 维护约定
//!
//! 各模块定义自己的错误枚举 (见各模块的 `error.rs`), 并在本模块用 [`PmSysdWError`]
//! 通过 `From` 汇总. 错误显示仅为尽力而为, 便于开发调试.

use std::{error::Error, fmt};

#[cfg(feature = "cli_ctl")]
use crate::cli::ctl::CtlError;
#[cfg(feature = "cli_init")]
use crate::cli::init::InitError;
#[cfg(feature = "cli_log")]
use crate::cli::log::LogError;

use crate::conf::ConfError;
use crate::db::DbError;
use crate::jsonl::JsonlError;
use crate::logd::LogdError;
use crate::pa::PaError;
use crate::sysd::SysdError;

#[cfg(target_os = "linux")]
use crate::p_u::PuError;
#[cfg(target_os = "windows")]
use crate::p_w::PwError;

/// pm-sysd-w 顶层错误.
#[derive(Debug, Clone)]
pub enum PmSysdWError {
    /// conf 错误
    Conf(ConfError),
    /// db 错误
    Db(DbError),
    /// jsonl 错误
    Jsonl(JsonlError),
    /// sysd 错误
    Sysd(SysdError),
    /// logd 错误
    Logd(LogdError),
    /// cli::init 错误
    #[cfg(feature = "cli_init")]
    Init(InitError),
    /// cli::ctl 错误
    #[cfg(feature = "cli_ctl")]
    Ctl(CtlError),
    /// cli::log 错误
    #[cfg(feature = "cli_log")]
    Log(LogError),
    /// pa 错误
    Pa(PaError),
    /// p_w (Windows) 错误
    #[cfg(target_os = "windows")]
    Pw(PwError),
    /// p_u (Linux) 错误
    #[cfg(target_os = "linux")]
    Pu(PuError),
    /// 顶层通用错误
    Msg(String),
}

impl Error for PmSysdWError {}

impl fmt::Display for PmSysdWError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conf(v) => write!(f, "{}", v),
            Self::Db(v) => write!(f, "{}", v),
            Self::Jsonl(v) => write!(f, "{}", v),
            Self::Sysd(v) => write!(f, "{}", v),
            Self::Logd(v) => write!(f, "{}", v),
            #[cfg(feature = "cli_init")]
            Self::Init(v) => write!(f, "{}", v),
            #[cfg(feature = "cli_ctl")]
            Self::Ctl(v) => write!(f, "{}", v),
            #[cfg(feature = "cli_log")]
            Self::Log(v) => write!(f, "{}", v),
            Self::Pa(v) => write!(f, "{}", v),
            #[cfg(target_os = "windows")]
            Self::Pw(v) => write!(f, "{}", v),
            #[cfg(target_os = "linux")]
            Self::Pu(v) => write!(f, "{}", v),
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}

impl From<ConfError> for PmSysdWError {
    fn from(value: ConfError) -> Self {
        Self::Conf(value)
    }
}

impl From<DbError> for PmSysdWError {
    fn from(value: DbError) -> Self {
        Self::Db(value)
    }
}

impl From<JsonlError> for PmSysdWError {
    fn from(value: JsonlError) -> Self {
        Self::Jsonl(value)
    }
}

impl From<SysdError> for PmSysdWError {
    fn from(value: SysdError) -> Self {
        Self::Sysd(value)
    }
}

impl From<LogdError> for PmSysdWError {
    fn from(value: LogdError) -> Self {
        Self::Logd(value)
    }
}

#[cfg(feature = "cli_init")]
impl From<InitError> for PmSysdWError {
    fn from(value: InitError) -> Self {
        Self::Init(value)
    }
}

#[cfg(feature = "cli_ctl")]
impl From<CtlError> for PmSysdWError {
    fn from(value: CtlError) -> Self {
        Self::Ctl(value)
    }
}

#[cfg(feature = "cli_log")]
impl From<LogError> for PmSysdWError {
    fn from(value: LogError) -> Self {
        Self::Log(value)
    }
}

impl From<PaError> for PmSysdWError {
    fn from(value: PaError) -> Self {
        Self::Pa(value)
    }
}

#[cfg(target_os = "windows")]
impl From<PwError> for PmSysdWError {
    fn from(value: PwError) -> Self {
        Self::Pw(value)
    }
}

#[cfg(target_os = "linux")]
impl From<PuError> for PmSysdWError {
    fn from(value: PuError) -> Self {
        Self::Pu(value)
    }
}
