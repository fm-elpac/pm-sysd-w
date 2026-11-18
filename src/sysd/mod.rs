//! 服务管理核心逻辑.
//!
//! 管理由 pm-sysd-init 启动的进程 (process).
//! 初版只支持 service 一种 unit 类型.
//!
//! 采用多 actor 架构: 每个关键组件是 `tokio::spawn` 的 async task, 持有自己的内部状态.
//! 组件之间通过 channel 发送消息通信 (事件驱动).
//!
//! systemd 风格的状态定义与打印工具已移到 [`crate::sysu`].

pub mod init;
pub mod reload;

mod error;
mod service_d;
mod service_s;
mod sys_d;
mod timer_d;
mod timer_s;
mod unit_d;
mod util;

#[cfg(test)]
mod test_util;

pub use error::SysdError;
pub use sys_d::{SysD, SysDM};
pub use util::send_reply;

/// 内置 target: `default.target`
///
/// 平时正常运行时的 target.
pub const TARGET_DEFAULT: &str = "default.target";
/// 内置 target: `exit.target`
///
/// 退出时的 target.
pub const TARGET_EXIT: &str = "exit.target";
