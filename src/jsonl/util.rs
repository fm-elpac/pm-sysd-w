//! 通用 编解码 函数.

use serde::{Serialize, de::DeserializeOwned};

use super::error::JsonlError;

/// 把消息编码为 单行 JSON (`T` 为 `SdReq` 或 `SdRes`).
pub fn encode_frame<T: Serialize>(f: &T) -> Result<String, JsonlError> {
    serde_json::to_string(f).map_err(|e| JsonlError::Serde(e.to_string()))
}

/// 从 单行 JSON 解析消息 (`T` 为 `SdReq` 或 `SdRes`).
pub fn decode_frame<T: DeserializeOwned>(line: &str) -> Result<T, JsonlError> {
    serde_json::from_str(line).map_err(|e| JsonlError::Serde(e.to_string()))
}
