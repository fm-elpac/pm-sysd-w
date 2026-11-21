//! Linux 平台 (pa) 实现.
//!
//! 仅用于方便测试 (Linux 上已有 systemd, pm-sysd-w 几乎无用).
//!
//! IPC 使用 UNIX socket. 本模块只包含平台相关代码, 不做抽象/封装.

use std::{env, os::unix::process::CommandExt, process::Command};

pub mod ipc;
pub mod text;

mod error;

pub use error::PuError;
pub use ipc::{ClientStream, Listener, ServerStream, connect, exists};
pub use text::TextDecoder;

/// 重启自身 (soft-reboot): `exec` 替换当前进程, 失败时返回错误
pub fn soft_reboot(args: &[String]) -> Result<(), PuError> {
    let exe = env::current_exe().map_err(|e| PuError::Msg(format!("current_exe: {}", e)))?;
    let e = Command::new(&exe).args(args).exec();
    Err(PuError::Msg(format!("soft-reboot exec: {}", e)))
}
