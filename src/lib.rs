//! pm-sysd-w: 胖喵稳哒.
//!
//! Systemd style on Windows.
//!
//! 模仿 systemd 的风格与使用体验, 但只实现其中的一小部分功能 (subset).
#![deny(unsafe_code)]

#[cfg(feature = "cli")]
pub mod cli;

pub mod conf;
pub mod db;
pub mod error;
pub mod jsonl;
pub mod logd;
pub mod pa;
pub mod sysd;

#[cfg(target_os = "windows")]
pub mod p_w;

#[cfg(target_os = "linux")]
pub mod p_u;

#[cfg(test)]
mod testutil;

/// 当前编译目标 的 平台实现 类型 (由 条件编译 确定).
#[cfg(target_os = "linux")]
pub type Platform = crate::p_u::PaU;

/// 当前编译目标 的 平台实现 类型 (由 条件编译 确定).
#[cfg(target_os = "windows")]
pub type Platform = crate::p_w::PaW;

pub use error::PmSysdWError;
