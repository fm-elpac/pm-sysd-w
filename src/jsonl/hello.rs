//! 握手消息 (init 与 ctl 建立连接时交换).

use serde::{Deserialize, Serialize};

use super::error::JsonlError;

/// 传输协议格式版本 (信封结构版本).
pub const PROTOCOL_VERSION: u32 = 1;

/// pm-sysd-w 软件版本号 (来自 `Cargo.toml` 的 `version`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 握手消息 (外层信封).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    /// 传输协议格式版本, 固定为 [`PROTOCOL_VERSION`].
    #[serde(rename = "pm-sysd-w")]
    pub protocol: u32,
    /// 消息内容.
    pub data: HelloData,
}

/// 握手消息 `data` 内容.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloData {
    /// pm-sysd-w 软件版本号 (用于 init/ctl 版本检查).
    pub version: String,
}

impl Hello {
    /// 构造握手消息 (使用本 crate 的软件版本号 [`VERSION`]).
    pub fn new() -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            data: HelloData {
                version: VERSION.to_string(),
            },
        }
    }
}

/// 把握手消息编码为 单行 JSON.
pub fn encode_hello(h: &Hello) -> Result<String, JsonlError> {
    serde_json::to_string(h).map_err(|e| JsonlError::Serde(e.to_string()))
}

/// 从 单行 JSON 解析握手消息.
pub fn decode_hello(line: &str) -> Result<Hello, JsonlError> {
    serde_json::from_str(line).map_err(|e| JsonlError::Serde(e.to_string()))
}

/// 检查对方握手消息与自己是否兼容 (协议版本 与 软件版本 一致).
pub fn check_version(h: &Hello) -> Result<(), JsonlError> {
    if h.protocol != PROTOCOL_VERSION {
        return Err(JsonlError::Version(format!(
            "protocol version `{}` != `{}`",
            h.protocol, PROTOCOL_VERSION
        )));
    }
    if h.data.version != VERSION {
        return Err(JsonlError::Version(format!(
            "software version `{}` != `{}`",
            h.data.version, VERSION
        )));
    }
    Ok(())
}
