//! cli::log 模块错误.

use std::{error::Error, fmt};

/// log 模块错误.
#[derive(Debug, Clone)]
pub enum LogError {
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for LogError {}

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
