//! Windows 平台 (pa) 实现.
//!
//! 主要支持平台.
//!
//! IPC 使用 命名管道 (Named pipe): 只允许本机程序连接 (默认 `reject_remote_clients`),
//! 禁止网络连接. 使用 `first_pipe_instance` 进行实例唯一性检测.

pub mod console;
mod error;
#[cfg(test)]
mod test;

pub use error::PwError;

use std::time::Duration;

use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::pa::{BoxFuture, BoxStream, Pa, PaError, PaListener, SpawnOptions};

/// Windows 平台实现.
#[derive(Debug, Clone, Default)]
pub struct PaW;

impl Pa for PaW {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn spawn_flags(&self, cmd: &mut tokio::process::Command, opts: SpawnOptions) {
        if opts.create_no_window {
            cmd.creation_flags(CREATE_NO_WINDOW.0);
        }
    }

    fn ipc_listen(&self, path: &str) -> Result<Box<dyn PaListener>, PaError> {
        Ok(Box::new(PipeListener::new(path)?))
    }

    fn ipc_connect(&self, path: &str) -> BoxFuture<'static, Result<BoxStream, PaError>> {
        let pipe_name = pipe_addr(path);
        Box::pin(async move {
            // 简单实现: 连接 失败时 短暂重试 (服务器 可能在 替换 pipe 实例).
            let mut last = None;
            for _ in 0..100 {
                match ClientOptions::new().open(&pipe_name) {
                    Ok(c) => return Ok(Box::new(c) as BoxStream),
                    Err(e) => last = Some(e),
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(PaError::Ipc(format!(
                "connect pipe `{}`: {:?}",
                pipe_name, last
            )))
        })
    }
}

/// 命名管道 完整地址 (`\\.\pipe\NAME`).
fn pipe_addr(name: &str) -> String {
    format!(r"\\.\pipe\{}", name)
}

/// 创建 一个 命名管道 服务器.
fn create_server(addr: &str, first: bool) -> Result<NamedPipeServer, PaError> {
    let mut opts = ServerOptions::new();
    // `first_pipe_instance(true)` 用于 实例唯一性检测.
    opts.first_pipe_instance(first);
    opts.create(addr)
        .map_err(|e| PaError::Ipc(format!("create pipe `{}`: {}", addr, e)))
}

/// Windows 命名管道 监听器.
///
/// 使用 tokio 文档推荐的 模式: 始终保有 一个 未连接 的 server 实例, 避免 客户端
/// 连接时 出现 `NotFound`.
struct PipeListener {
    /// 完整管道地址.
    addr: String,
    /// 当前等待连接的 server 实例.
    pending: Option<NamedPipeServer>,
}

impl PipeListener {
    fn new(name: &str) -> Result<Self, PaError> {
        let addr = pipe_addr(name);
        // 第一个实例 使用 `first_pipe_instance(true)` 检测 是否已有同名实例.
        let pending = create_server(&addr, true)?;
        Ok(Self {
            addr,
            pending: Some(pending),
        })
    }
}

impl PaListener for PipeListener {
    fn accept<'a>(&'a mut self) -> BoxFuture<'a, Result<BoxStream, PaError>> {
        let server = match self.pending.take() {
            Some(s) => s,
            None => {
                return Box::pin(async {
                    Err(PaError::Ipc("pipe listener exhausted".to_string()))
                });
            }
        };

        // 先创建 替代 server (非 first), 保证 连接窗口 不中断.
        match create_server(&self.addr, false) {
            Ok(next) => self.pending = Some(next),
            Err(e) => {
                return Box::pin(async move { Err(e) });
            }
        }

        Box::pin(async move {
            server
                .connect()
                .await
                .map_err(|e| PaError::Ipc(format!("pipe connect: {}", e)))?;
            Ok(Box::new(server) as BoxStream)
        })
    }
}
