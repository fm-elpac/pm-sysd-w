//! cli::ctl 模块错误.

use std::{error::Error, fmt};

/// ctl 模块错误.
#[derive(Debug, Clone)]
pub enum CtlError {
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for CtlError {}

impl fmt::Display for CtlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
