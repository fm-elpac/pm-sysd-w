//! 底层平台 (操作系统) 抽象接口.
//!
//! Windows 实现见 `crate::p_w`, Linux 实现见 `crate::p_u` (仅用于测试).
//!
//! 抽象两部分差异:
//!
//! - 进程 (service) 管理: 启动 / 停止.
//! - IPC 连接: Windows 命名管道 / Linux UNIX socket.

mod error;
#[cfg(test)]
mod test;

pub use error::PaError;

use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
};

use tokio::{
    io::{AsyncRead, AsyncWrite},
    process::{Child, Command},
};

/// 双向字节流 (IPC 连接).
pub trait PaStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> PaStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

/// 装箱的 双向字节流.
pub type BoxStream = Box<dyn PaStream>;

/// 装箱的 Future.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 进程启动选项.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpawnOptions {
    /// Windows: 使用 CREATE_NO_WINDOW 标志 启动 (console 程序不显示 黑窗口).
    ///
    /// 仅 Windows 平台有效, 其它平台 忽略.
    pub create_no_window: bool,
}

/// IPC 监听器 (接受连接).
pub trait PaListener: Send {
    /// 接受下一个连接.
    fn accept<'a>(&'a mut self) -> BoxFuture<'a, Result<BoxStream, PaError>>;
}

/// 平台抽象接口: 封装 Windows / Linux 的差异.
pub trait Pa: Send + Sync {
    /// 平台名称 (用于显示 / 调试).
    fn name(&self) -> &'static str;

    /// 启动进程 (返回 tokio `Child`).
    ///
    /// 子进程的 stdout/stderr 以管道形式接管 (供上层读取并写入日志).
    ///
    /// 默认实现 为 跨平台 的 [`tokio::process::Command`], 平台可实现 覆盖.
    fn spawn(
        &self,
        cmd: &[String],
        cwd: Option<&Path>,
        env: &[(String, String)],
        opts: SpawnOptions,
    ) -> Result<Child, PaError> {
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..]);
        c.stdout(Stdio::piped());
        c.stderr(Stdio::piped());
        if let Some(d) = cwd {
            c.current_dir(d);
        }
        for (k, v) in env {
            c.env(k, v);
        }
        self.spawn_flags(&mut c, opts);
        // 进程 意外被丢弃时 一并终止 (避免 僵尸进程).
        c.kill_on_drop(true);
        c.spawn()
            .map_err(|e| PaError::Process(format!("spawn `{}`: {}", cmd[0], e)))
    }

    /// 平台特定 的 进程创建 标志 (默认 无).
    fn spawn_flags(&self, _cmd: &mut Command, _opts: SpawnOptions) {}

    /// 绑定 IPC 监听 (同时用于 实例唯一性检测).
    fn ipc_listen(&self, path: &str) -> Result<Box<dyn PaListener>, PaError>;

    /// 连接 IPC 服务器.
    fn ipc_connect(&self, path: &str) -> BoxFuture<'static, Result<BoxStream, PaError>>;
}

/// pm-sysd-w 实例 (决定 IPC 地址).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instance {
    /// 系统级实例 (`--system`).
    System,
    /// 用户级实例 (`--user`).
    User,
    /// 自定义实例 (`--machine=XXX`).
    Machine(String),
}

impl Instance {
    /// 实例名称 (不含 平台路径前缀).
    pub fn name(&self) -> String {
        match self {
            Self::System => "pm_sysd_w_system_default".to_string(),
            Self::User => format!("pm_sysd_w_user_{}", current_user()),
            Self::Machine(m) => format!("pm_sysd_w_m_{}", m),
        }
    }

    /// IPC 传输地址.
    ///
    /// - Linux: 当前目录 的 UNIX socket 文件.
    /// - Windows: 命名管道 名称.
    pub fn transport(&self) -> String {
        let name = self.name();
        #[cfg(target_os = "linux")]
        {
            format!("./{}.sock", name)
        }
        #[cfg(target_os = "windows")]
        {
            name
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            name
        }
    }
}

/// 当前用户名 (实例命名用).
#[cfg(target_os = "linux")]
fn current_user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "unknown".to_string())
}

/// 当前用户名 (实例命名用).
#[cfg(target_os = "windows")]
fn current_user() -> String {
    std::env::var("USERNAME").unwrap_or_else(|_| "unknown".to_string())
}

/// 当前用户名 (实例命名用).
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn current_user() -> String {
    "unknown".to_string()
}

/// 把 命令字符串 拆分为 程序路径 + 参数 列表.
///
/// 简单实现: 按空白切分, 支持 双引号 (`"..."`) 分组 与 反斜杠 (`\`) 转义.
/// 初版足够用于 测试; Windows 完整命令行解析 后续 补充.
pub fn split_cmd(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    let mut chars = cmd.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\\' if !in_quote => {
                if let Some(&next) = chars.peek() {
                    cur.push(next);
                    chars.next();
                } else {
                    cur.push('\\');
                }
            }
            '"' => in_quote = !in_quote,
            c if c.is_whitespace() && !in_quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// 相对路径 转 绝对路径 (基于 `base`).
pub fn resolve_path(base: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}
