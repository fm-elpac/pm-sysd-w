//! p_w 模块错误.

use std::{error::Error, fmt};

/// p_w 模块错误.
#[derive(Debug, Clone)]
pub enum PwError {
    /// IPC 错误
    Ipc(String),
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for PwError {}

impl fmt::Display for PwError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ipc(m) => write!(f, "p_w: {}", m),
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
