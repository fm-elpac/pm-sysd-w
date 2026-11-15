//! 握手消息 (init 与 ctl 建立连接时交换).

use std::fmt;

use serde::{Deserialize, Serialize};

use super::error::JsonlError;

/// 传输协议格式版本 (信封结构版本)
pub const PROTOCOL_VERSION: u32 = 1;

/// pm-sysd-w 软件版本号 (来自 `Cargo.toml` 的 `version`)
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 握手消息 (外层信封)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    /// 传输协议格式版本, 固定为 [`PROTOCOL_VERSION`]
    #[serde(rename = "pm-sysd-w")]
    pub protocol: u32,
    /// 消息内容
    pub data: HelloData,
}

/// 握手消息 `data` 内容
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloData {
    /// pm-sysd-w 软件版本号 (用于 init/ctl 版本检查)
    pub version: String,
}

impl Hello {
    /// 构造握手消息 (使用本 crate 的软件版本号 [`VERSION`])
    pub fn new() -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            data: HelloData {
                version: VERSION.to_string(),
            },
        }
    }
}

/// 把握手消息编码为单行 JSON
pub fn encode_hello(h: &Hello) -> Result<String, JsonlError> {
    serde_json::to_string(h).map_err(|e| JsonlError::Serde(e.to_string()))
}

/// 从单行 JSON 解析握手消息
pub fn decode_hello(line: &str) -> Result<Hello, JsonlError> {
    serde_json::from_str(line).map_err(|e| JsonlError::Serde(e.to_string()))
}

/// 握手消息版本不一致详情.
///
/// 用于生成面向用户的友好报错: 明确是哪种版本不一致, 并显示双方具体版本号.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionMismatch {
    /// 传输协议格式版本不一致
    Protocol {
        /// 本进程的协议版本
        local: u32,
        /// 对方的协议版本
        remote: u32,
    },
    /// 软件版本号不一致
    Software {
        /// 本进程的软件版本号
        local: String,
        /// 对方的软件版本号
        remote: String,
    },
}

impl fmt::Display for VersionMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol { local, remote } => write!(
                f,
                "protocol version mismatch: local `{}`, remote `{}`",
                local, remote
            ),
            Self::Software { local, remote } => write!(
                f,
                "software version mismatch: local `{}`, remote `{}`",
                local, remote
            ),
        }
    }
}

/// 检查对方握手消息与自己是否兼容 (协议版本与软件版本一致)
pub fn check_version(h: &Hello) -> Result<(), VersionMismatch> {
    if h.protocol != PROTOCOL_VERSION {
        return Err(VersionMismatch::Protocol {
            local: PROTOCOL_VERSION,
            remote: h.protocol,
        });
    }
    if h.data.version != VERSION {
        return Err(VersionMismatch::Software {
            local: VERSION.to_string(),
            remote: h.data.version.clone(),
        });
    }
    Ok(())
}
