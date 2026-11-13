//! init 与 ctl 之间的 JSONL 通信协议.
//!
//! 传输基于 JSONL (单行 JSON), 使用 `serde_json` 编解码.
//!
//! 握手消息格式 (外层信封 + `data`):
//!
//! ```json
//! {"pm-sysd-w":1,"data":{"version":"0.1.0-a1"}}
//! ```
//!
//! - `pm-sysd-w`: 传输协议格式版本 (信封结构版本), 当前固定为 1.
//! - `data.version`: pm-sysd-w 软件版本号 (字符串, semver), 用于版本检查.
//!
//! 握手后, 后续消息无需版本号. 后续消息为 [`SdReq`] (ctl -> init) 或
//! [`SdRes`] (init -> ctl), 二者均为结构化数据.
//!
//! 本模块只负责消息编解码 (协议处理), 不做 IO.
//! 消息收发 (含单条消息最大长度限制与读/写超时) 由 `pa` 模块负责.

mod error;
mod hello;
mod req;
mod res;
mod util;

#[cfg(test)]
mod test;

pub use error::JsonlError;
pub use hello::{
    Hello, HelloData, PROTOCOL_VERSION, VERSION, check_version, decode_hello, encode_hello,
};
pub use req::{
    SdReq, SdReqLogQuery, SdReqLogRO, SdReqSysd, SdReqUnit, SdReqUnitOp, SdReqUnitOpRO,
    SdReqUnitRO, SdReqUnitd, SdReqUnitdRO,
};
pub use res::{
    SdRes, SdResLogEntry, SdResServiceStatus, SdResTimerInfo, SdResTimerStatus, SdResUnitS,
    SdResUnitState, SdResUnitStatus,
};
pub use util::{decode_frame, encode_frame};
