//! Windows 控制台自适应.
//!
//! 本模块是 p_w 中允许 `unsafe` 的模块之一, 仅限 Win32 API 边界.
#![allow(unsafe_code)]

use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList, GetConsoleWindow};

/// 控制台自适应.
///
/// 从已有控制台启动时保持现状. 若系统自动分配了控制台 (例如双击启动 /
/// 无控制台), 则断开此控制台, 避免一直显示黑窗口.
pub fn adjust_console() {
    // 无控制台窗口时无需处理 (不会出现黑窗口)
    if unsafe { GetConsoleWindow() }.is_invalid() {
        return;
    }
    // 若控制台只挂载了本进程 (即系统自动创建), 断开该控制台
    let mut pids = [0u32; 2];
    let n = unsafe { GetConsoleProcessList(&mut pids) };
    if n == 1 {
        // 断开控制台失败仅影响黑窗口显示, 属尽力而为. 失败忽略.
        let _ = unsafe { FreeConsole() };
    }
}
