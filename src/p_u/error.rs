//! p_u 模块错误.

use std::{error::Error, fmt};

/// p_u 模块错误.
#[derive(Debug, Clone)]
pub enum PuError {
    /// IPC 错误
    Ipc(String),
    /// IPC 端点 不存在
    NotFound(String),
    /// IPC 权限不足
    Permission(String),
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for PuError {}

impl fmt::Display for PuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ipc(m) => write!(f, "p_u: {}", m),
            Self::NotFound(m) => write!(f, "p_u: {}", m),
            Self::Permission(m) => write!(f, "p_u: {}", m),
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
