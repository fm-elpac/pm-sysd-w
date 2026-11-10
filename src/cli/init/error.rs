//! cli::init 模块错误.

use std::{error::Error, fmt};

/// init 模块错误.
#[derive(Debug, Clone)]
pub enum InitError {
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for InitError {}

impl fmt::Display for InitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
