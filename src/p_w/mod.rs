//! Windows 平台 (pa) 实现.
//!
//! 主要支持平台.
//!
//! IPC 使用 命名管道 (Named pipe): 只允许本机程序连接 (默认 `reject_remote_clients`),
//! 禁止网络连接. 使用 `first_pipe_instance` 进行实例唯一性检测.
//!
//! 本模块 只包含 平台相关 代码, 不做 抽象/封装.

use std::{env, process, time::Duration};

use tokio::{
    net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer},
    process::Command,
    time::{Instant, sleep},
};
use windows::Win32::{
    Foundation::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND},
    System::Threading::CREATE_NO_WINDOW,
};

use crate::pa::{IpcAcl, SpawnOptions};

pub mod acl;
pub mod console;

mod error;

#[cfg(test)]
mod test;

pub use error::PwError;

use acl::{PipeAcl, create_server, pipe_exists};

/// 客户端 连接流.
pub type ClientStream = NamedPipeClient;

/// 服务端 连接流.
pub type ServerStream = NamedPipeServer;

/// 命名管道 监听器.
///
/// 使用 tokio 文档推荐的模式: 始终保有 一个未连接的 server 实例, 避免客户端
/// 连接时出现 `NotFound`.
pub struct Listener {
    /// 完整管道地址.
    addr: String,
    /// 管道 DACL 权限级别.
    acl: PipeAcl,
    /// 当前等待连接的 server 实例.
    pending: Option<NamedPipeServer>,
}

impl Listener {
    /// 创建 命名管道 监听器.
    ///
    /// 第一个实例 使用 `first_pipe_instance(true)` 检测 是否已有同名实例.
    pub fn bind(name: &str, acl: PipeAcl) -> Result<Self, PwError> {
        let addr = pipe_addr(name);
        let pending = create_server(&addr, true, acl)?;
        Ok(Self {
            addr,
            acl,
            pending: Some(pending),
        })
    }

    /// 接受 下一个连接.
    pub async fn accept(&mut self) -> Result<ServerStream, PwError> {
        let server = match self.pending.take() {
            Some(s) => s,
            None => {
                return Err(PwError::Ipc("pipe listener exhausted".to_string()));
            }
        };

        // 先创建替代 server (非 first), 保证连接窗口不中断.
        match create_server(&self.addr, false, self.acl) {
            Ok(next) => self.pending = Some(next),
            Err(e) => {
                return Err(e);
            }
        }

        server
            .connect()
            .await
            .map_err(|e| PwError::Ipc(format!("pipe connect: {}", e)))?;
        Ok(server)
    }
}

/// 连接 命名管道.
///
/// 快速失败: 总耗时 最多 200ms. 服务器 可能在替换 pipe 实例, 仅在
/// ERROR_PIPE_BUSY 时 短暂重试; 不存在 / 权限不足 直接返回.
pub async fn connect(name: &str) -> Result<ClientStream, PwError> {
    let pipe_name = pipe_addr(name);
    let deadline = Instant::now() + Duration::from_millis(200);
    loop {
        match ClientOptions::new().open(&pipe_name) {
            Ok(c) => return Ok(c),
            Err(e) => {
                let code = e.raw_os_error();
                if code == Some(ERROR_FILE_NOT_FOUND.0 as i32) {
                    return Err(PwError::NotFound(format!(
                        "connect pipe `{}`: not found",
                        pipe_name
                    )));
                }
                if code == Some(ERROR_ACCESS_DENIED.0 as i32) {
                    return Err(PwError::Permission(format!(
                        "connect pipe `{}`: access denied",
                        pipe_name
                    )));
                }
                if Instant::now() >= deadline {
                    return Err(PwError::Ipc(format!(
                        "connect pipe `{}`: os error {:?}",
                        pipe_name, code
                    )));
                }
            }
        }
        sleep(Duration::from_millis(20)).await;
    }
}

/// 检查 命名管道 是否存在.
pub fn exists(name: &str) -> bool {
    pipe_exists(&pipe_addr(name))
}

/// 平台特定 的 进程创建 标志.
pub fn spawn_flags(cmd: &mut Command, opts: SpawnOptions) {
    if opts.create_no_window {
        cmd.creation_flags(CREATE_NO_WINDOW.0);
    }
}

/// 重启自身 (soft-reboot): 启动 新进程 后 当前进程 自行退出.
pub fn soft_reboot(args: &[String]) -> Result<(), PwError> {
    let exe = env::current_exe().map_err(|e| PwError::Msg(format!("current_exe: {}", e)))?;
    match process::Command::new(&exe).args(args).spawn() {
        Ok(_) => Ok(()),
        Err(e) => Err(PwError::Msg(format!("soft-reboot spawn: {}", e))),
    }
}

/// `IpcAcl` -> `PipeAcl`.
impl From<IpcAcl> for PipeAcl {
    fn from(acl: IpcAcl) -> Self {
        match acl {
            IpcAcl::System => PipeAcl::System,
            IpcAcl::SystemReadOnly => PipeAcl::SystemReadOnly,
            IpcAcl::CurrentUser => PipeAcl::CurrentUser,
        }
    }
}

/// 命名管道 完整地址 (`\\.\pipe\NAME`).
fn pipe_addr(name: &str) -> String {
    format!(r"\\.\pipe\{}", name)
}
