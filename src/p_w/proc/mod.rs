//! p_w::proc: Windows 子进程创建辅助.

use std::{env, process};

use tokio::process::Command;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::pa::SpawnOptions;

use super::PwError;

/// 平台特定的进程创建标志
pub fn spawn_flags(cmd: &mut Command, opts: SpawnOptions) {
    if opts.create_no_window {
        cmd.creation_flags(CREATE_NO_WINDOW.0);
    }
}

/// 重启自身 (soft-reboot): 启动新进程后当前进程自行退出
pub fn soft_reboot(args: &[String]) -> Result<(), PwError> {
    let exe = env::current_exe().map_err(|e| PwError::Msg(format!("current_exe: {}", e)))?;
    match process::Command::new(&exe).args(args).spawn() {
        Ok(_) => Ok(()),
        Err(e) => Err(PwError::Msg(format!("soft-reboot spawn: {}", e))),
    }
}
