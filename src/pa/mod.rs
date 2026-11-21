//! 底层平台 (操作系统) 抽象/封装.
//!
//! Windows 实现见 `crate::p_w`, Linux 实现见 `crate::p_u` (仅用于测试).
//!
//! 本模块使用条件编译选择具体平台实现 (`p`).
//! 其余模块只依赖本模块, 不直接使用 `p_u` / `p_w`.
//!
//! 抽象两部分差异:
//!
//! - 进程 (process) 管理: 启动 / 停止 / 读取 stdout / stderr (见 `proc`).
//! - IPC 连接: Windows 命名管道 / Linux UNIX socket (见 `ipc`).
//!
//! 启动 / 退出 相关封装见 `init`.

use std::{
    env,
    path::{Path, PathBuf},
};

#[cfg(target_os = "linux")]
use crate::p_u as p;

#[cfg(target_os = "windows")]
use crate::p_w as p;

mod error;
mod init;
mod ipc;
mod proc;

#[cfg(test)]
mod test;

pub use error::PaError;
pub use init::{self_service_name, soft_reboot, spawn_signal_watch};
pub use ipc::{Instance, IpcAcl, SysdClient, SysdConn, SysdListener, choose_transport, ipc_exists};
pub use proc::{NativeProc, SpawnOptions};

/// 流式文本解码器.
///
/// 连续输入多个字节缓冲区, 解码结果按字符边界返回; 未构成完整字符的字节
/// 保留在解码器内部, 下次输入继续解码.
///
/// - Linux: 固定按 UTF-8 解码.
/// - Windows: 先按 UTF-8 解码, 失败后切换到系统 ACP 模式; ACP 再失败则
///   返回错误并退出 ACP 模式.
pub struct TextDecoder {
    inner: p::TextDecoder,
}

impl TextDecoder {
    /// 创建新的解码器
    pub fn new() -> Self {
        Self {
            inner: p::TextDecoder::new(),
        }
    }

    /// 输入一个字节缓冲区, 返回已解码的字符串
    pub fn decode(&mut self, bytes: &[u8]) -> Result<String, PaError> {
        self.inner.decode(bytes).map_err(PaError::from)
    }

    /// 结束输入: 解码内部剩余字节
    pub fn finish(&mut self) -> Result<String, PaError> {
        self.inner.finish().map_err(PaError::from)
    }
}

/// 当前主机名 (日志显示用)
pub fn hostname() -> String {
    #[cfg(target_os = "linux")]
    {
        env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string())
    }
    #[cfg(windows)]
    {
        env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".to_string())
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        "localhost".to_string()
    }
}

/// 相对路径转绝对路径 (基于 `base`)
pub fn resolve_path(base: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

#[cfg(target_os = "linux")]
impl From<crate::p_u::PuError> for PaError {
    fn from(e: crate::p_u::PuError) -> Self {
        use crate::p_u::PuError as E;
        match e {
            E::Ipc(m) => PaError::Ipc(m),
            E::NotFound(m) => PaError::IpcNotFound(m),
            E::Permission(m) => PaError::IpcPermission(m),
            E::Msg(m) => PaError::Msg(m),
        }
    }
}

#[cfg(windows)]
impl From<crate::p_w::PwError> for PaError {
    fn from(e: crate::p_w::PwError) -> Self {
        use crate::p_w::PwError as E;
        match e {
            E::Ipc(m) => PaError::Ipc(m),
            E::NotFound(m) => PaError::IpcNotFound(m),
            E::Permission(m) => PaError::IpcPermission(m),
            E::Msg(m) => PaError::Msg(m),
        }
    }
}
