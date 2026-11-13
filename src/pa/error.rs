//! pa 模块错误.

use std::{error::Error, fmt};

/// pa 模块错误.
#[derive(Debug, Clone)]
pub enum PaError {
    /// 进程 错误
    Process(String),
    /// IPC 错误
    Ipc(String),
    /// IPC 端点 不存在 (服务器未运行).
    IpcNotFound(String),
    /// IPC 权限不足.
    IpcPermission(String),
    /// IO 错误 (消息收发).
    Io(String),
    /// 连接关闭 (EOF).
    Eof,
    /// 单条消息 超出最大长度.
    TooLong(String),
    /// 单条消息 收/发 超时.
    Timeout(String),
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for PaError {}

impl fmt::Display for PaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Process(m) => write!(f, "pa: {}", m),
            Self::Ipc(m) => write!(f, "pa: {}", m),
            Self::IpcNotFound(m) => write!(f, "pa: {}", m),
            Self::IpcPermission(m) => write!(f, "pa: {}", m),
            Self::Io(m) => write!(f, "pa: {}", m),
            Self::Eof => write!(f, "pa: connection closed"),
            Self::TooLong(m) => write!(f, "pa: {}", m),
            Self::Timeout(m) => write!(f, "pa: {}", m),
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
