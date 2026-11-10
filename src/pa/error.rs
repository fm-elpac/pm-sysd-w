//! pa 模块错误.

use std::{error::Error, fmt};

/// pa 模块错误.
#[derive(Debug, Clone)]
pub enum PaError {
    /// 进程 错误
    Process(String),
    /// IPC 错误
    Ipc(String),
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for PaError {}

impl fmt::Display for PaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Process(m) => write!(f, "pa: {}", m),
            Self::Ipc(m) => write!(f, "pa: {}", m),
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
