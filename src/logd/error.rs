//! logd 模块错误.

use std::{error::Error, fmt};

/// logd 模块错误.
#[derive(Debug, Clone)]
pub enum LogdError {
    /// 数据库 错误
    Db(String),
    /// 日志缓冲区 已满 (发送方 无阻塞投递失败)
    Busy(String),
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for LogdError {}

impl fmt::Display for LogdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Db(m) => write!(f, "logd: {}", m),
            Self::Busy(m) => write!(f, "logd: {}", m),
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
