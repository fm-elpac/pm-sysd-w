//! jsonl 模块错误.

use std::{error::Error, fmt};

/// jsonl 模块错误.
#[derive(Debug, Clone)]
pub enum JsonlError {
    /// 编解码 (serde_json) 错误
    Serde(String),
    /// 协议版本 / 软件版本 不一致
    Version(String),
    /// 连接关闭 (EOF)
    Eof,
    /// IO 错误
    Io(String),
    /// 非法消息
    Bad(String),
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for JsonlError {}

impl fmt::Display for JsonlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serde(m) => write!(f, "jsonl: {}", m),
            Self::Version(m) => write!(f, "jsonl: {}", m),
            Self::Eof => write!(f, "jsonl: connection closed"),
            Self::Io(m) => write!(f, "jsonl: {}", m),
            Self::Bad(m) => write!(f, "jsonl: {}", m),
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
