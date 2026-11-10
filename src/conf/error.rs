//! conf 模块错误.

use std::{error::Error, fmt};

/// conf 模块错误.
#[derive(Debug, Clone)]
pub enum ConfError {
    /// pm-tc 配置解析 / 计划 错误
    Pm(String),
    /// unit 配置 错误
    BadUnit(String),
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for ConfError {}

impl fmt::Display for ConfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pm(m) => write!(f, "conf: {}", m),
            Self::BadUnit(m) => write!(f, "conf: {}", m),
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
