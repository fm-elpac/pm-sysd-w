//! sysd 模块错误.

use std::{error::Error, fmt};

/// sysd 模块错误.
#[derive(Debug, Clone)]
pub enum SysdError {
    /// unit 未找到 / 未加载
    NotFound(String),
    /// 操作 失败 (携带消息)
    Failed(String),
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for SysdError {}

impl fmt::Display for SysdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(m) => write!(f, "sysd: {}", m),
            Self::Failed(m) => write!(f, "sysd: {}", m),
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
