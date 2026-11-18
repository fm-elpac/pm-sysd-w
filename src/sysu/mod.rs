//! sysu 模块: 从 systemd 模仿而来的定义/风格与小功能.
//!
//! - [`s`]: 状态定义 (`UnitSta` 等).
//! - [`ctl`]: pm-sysd-ctl 使用的 systemctl 风格输出与退出码.
//! - [`log`]: pm-sysd-log 使用的 journalctl 风格输出.
//! - `p` 内部模块: systemd 风格的打印输出 / 字符串格式化, 通过 `pub use` 导出.

pub mod ctl;
pub mod log;
pub mod s;

mod p;

pub use p::reached_target;
pub use s::{EnableSta, LoadSta, ServiceSta, TargetSta, TimerSta, UnitSta, UnitSubSta};

#[cfg(test)]
mod test;
