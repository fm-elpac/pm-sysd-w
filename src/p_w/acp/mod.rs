//! Windows 系统代码页 (ACP) 获取.
//!
//! 本模块是 p_w 中 允许 `unsafe` 的模块之一, 仅封装 `GetACP` 调用.
#![allow(unsafe_code)]

use windows::Win32::Globalization::GetACP;

/// 获取系统 ANSI 代码页 (ACP)
pub fn get_acp() -> u16 {
    (unsafe { GetACP() }) as u16
}
