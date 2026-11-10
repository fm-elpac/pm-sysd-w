//! 命令行程序 (CLI) 模块.
//!
//! 仅启用 `cli` feature 时 编译.
//!
//! 包含 3 个二进制程序 的 CLI 实现:
//!
//! - [`init`]: pm-sysd-init 程序.
//! - [`ctl`]: pm-sysd-ctl 程序.
//! - [`log`]: pm-sysd-log 程序.

#[cfg(feature = "cli_init")]
pub mod init;

#[cfg(feature = "cli_ctl")]
pub mod ctl;

#[cfg(feature = "cli_log")]
pub mod log;
