//! Windows 平台 (pa) 实现.
//!
//! 主要支持平台.
//!
//! IPC 使用 命名管道 (Named pipe): 只允许本机程序连接 (默认 `reject_remote_clients`),
//! 禁止网络连接. 使用 `first_pipe_instance` 进行实例唯一性检测.
//!
//! 本模块只包含平台相关代码, 不做抽象/封装.

pub mod acl;
pub mod acp;
pub mod console;
pub mod ipc;
pub mod proc;
pub mod service;
pub mod text;

mod error;

pub use error::PwError;
pub use ipc::{ClientStream, Listener, ServerStream, connect, exists};
pub use proc::{soft_reboot, spawn_flags};
pub use text::TextDecoder;
