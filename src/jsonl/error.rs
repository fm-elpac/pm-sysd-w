//! jsonl 模块错误.

use std::{error::Error, fmt};

/// jsonl 模块错误.
///
/// 只包含 编解码 / 版本检查 相关错误; 消息收发 IO 错误 见 `pa::PaError`.
#[derive(Debug, Clone)]
pub enum JsonlError {
    /// 编解码 (serde_json) 错误
    Serde(String),
    /// 协议版本 / 软件版本 不一致
    Version(String),
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for JsonlError {}

impl fmt::Display for JsonlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serde(m) => write!(f, "jsonl: {}", m),
            Self::Version(m) => write!(f, "jsonl: {}", m),
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
