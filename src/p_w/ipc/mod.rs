//! p_w::ipc: Windows 命名管道 IPC.

use std::time::Duration;

use tokio::{
    net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer},
    time::{Instant, sleep},
};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND};

use crate::pa::IpcAcl;

use super::{
    PwError,
    acl::{PipeAcl, create_server, pipe_exists},
};

/// 客户端连接流
pub type ClientStream = NamedPipeClient;

/// 服务端连接流
pub type ServerStream = NamedPipeServer;

/// 命名管道监听器.
///
/// 使用 tokio 文档推荐的模式: 始终保有一个未连接的 server 实例, 避免客户端
/// 连接时出现 `NotFound`.
pub struct Listener {
    /// 完整管道地址
    addr: String,
    /// 管道 DACL 权限级别
    acl: PipeAcl,
    /// 当前等待连接的 server 实例
    pending: Option<NamedPipeServer>,
}

impl Listener {
    /// 创建命名管道监听器.
    ///
    /// 第一个实例使用 `first_pipe_instance(true)` 检测是否已有同名实例.
    pub fn bind(name: &str, acl: PipeAcl) -> Result<Self, PwError> {
        let addr = pipe_addr(name);
        let pending = create_server(&addr, true, acl)?;
        Ok(Self {
            addr,
            acl,
            pending: Some(pending),
        })
    }

    /// 接受下一个连接
    pub async fn accept(&mut self) -> Result<ServerStream, PwError> {
        let server = match self.pending.take() {
            Some(s) => s,
            None => {
                return Err(PwError::Ipc("pipe listener exhausted".to_string()));
            }
        };

        // 先创建替代 server (非 first), 保证连接窗口不中断
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

/// 连接命名管道.
///
/// 快速失败: 总耗时最多由 `SysdClientLimit::ipc_connect_timeout` 决定.
/// 服务器可能在替换 pipe 实例, 仅在 ERROR_PIPE_BUSY 时按配置的间隔短暂重试;
/// 不存在 / 权限不足直接返回.
pub async fn connect(
    name: &str,
    connect_timeout: Duration,
    retry_interval: Duration,
) -> Result<ClientStream, PwError> {
    let pipe_name = pipe_addr(name);
    let deadline = Instant::now() + connect_timeout;
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
        sleep(retry_interval).await;
    }
}

/// 检查命名管道是否存在
pub fn exists(name: &str) -> bool {
    pipe_exists(&pipe_addr(name))
}

/// `IpcAcl` -> `PipeAcl`
impl From<IpcAcl> for PipeAcl {
    fn from(acl: IpcAcl) -> Self {
        match acl {
            IpcAcl::System => PipeAcl::System,
            IpcAcl::SystemReadOnly => PipeAcl::SystemReadOnly,
            IpcAcl::CurrentUser => PipeAcl::CurrentUser,
        }
    }
}

/// 命名管道完整地址 (`\\.\pipe\NAME`)
pub(super) fn pipe_addr(name: &str) -> String {
    format!(r"\\.\pipe\{}", name)
}

#[cfg(test)]
mod test;
