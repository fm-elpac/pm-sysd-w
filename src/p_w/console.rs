//! Windows 控制台 自适应.
//!
//! 本模块 是 p_w 中 唯一 允许 `unsafe` 的模块 (仅限 Win32 API 边界).

#![allow(unsafe_code)]

use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList, GetConsoleWindow};

/// 控制台 自适应.
///
/// 从 已有控制台 启动时 保持现状; 若 系统自动分配了 控制台 (例如 双击启动 /
/// 无父控制台), 则 断开 该控制台, 避免 一直显示 黑窗口.
pub fn adjust_console() {
    // 无控制台窗口 时 无需处理 (不会出现 黑窗口).
    if unsafe { GetConsoleWindow() }.is_invalid() {
        return;
    }
    // 若 控制台 只挂载了 本进程 (即 系统自动创建), 断开 该控制台.
    let mut pids = [0u32; 2];
    let n = unsafe { GetConsoleProcessList(&mut pids) };
    if n == 1 {
        // 断开失败 属 尽力而为, 忽略.
        let _ = unsafe { FreeConsole() };
    }
}
