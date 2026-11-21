//! 子进程 管理: 启动 / 停止 / 读取 stdout / stderr.

use std::{path::Path, process::Stdio};

use tokio::process::{Child, ChildStderr, ChildStdout, Command};

#[cfg(windows)]
use crate::p_w as p;

use super::PaError;

/// 进程启动选项
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpawnOptions {
    /// Windows: 使用 CREATE_NO_WINDOW 标志启动 (console 程序不显示黑窗口)
    ///
    /// 仅 Windows 平台有效, 其它平台忽略
    pub create_no_window: bool,
}

/// 子进程封装.
///
/// stdout / stderr 以管道形式接管 (供上层读取并写入日志).
pub struct NativeProc {
    child: Child,
}

impl NativeProc {
    /// 启动进程
    pub fn spawn(
        argv: &[String],
        cwd: Option<&Path>,
        env: &[(String, String)],
        opts: SpawnOptions,
    ) -> Result<Self, PaError> {
        if argv.is_empty() {
            return Err(PaError::Process("spawn: empty command".into()));
        }

        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);

        c.stdout(Stdio::piped());
        c.stderr(Stdio::piped());

        if let Some(d) = cwd {
            c.current_dir(d);
        }
        for (k, v) in env {
            c.env(k, v);
        }

        #[cfg(windows)]
        p::spawn_flags(&mut c, opts);
        // Linux 不使用 opts (仅 Windows 用), 标记为已用避免 unused 警告.
        #[cfg(target_os = "linux")]
        let _ = opts;

        // 进程意外被丢弃时一并终止 (避免僵尸进程)
        c.kill_on_drop(true);

        let child = c
            .spawn()
            .map_err(|e| PaError::Process(format!("spawn `{}`: {}", argv[0], e)))?;
        Ok(Self { child })
    }

    /// 子进程 pid
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// 终止子进程
    pub async fn kill(&mut self) -> Result<(), PaError> {
        self.child
            .kill()
            .await
            .map_err(|e| PaError::Process(e.to_string()))
    }

    /// 等待子进程退出
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus, PaError> {
        self.child
            .wait()
            .await
            .map_err(|e| PaError::Process(e.to_string()))
    }

    /// 检查子进程是否已退出 (非阻塞)
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>, PaError> {
        self.child
            .try_wait()
            .map_err(|e| PaError::Process(e.to_string()))
    }

    /// 取出 stdout 管道 (仅成功启动后有效)
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    /// 取出 stderr 管道 (仅成功启动后有效)
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }
}
