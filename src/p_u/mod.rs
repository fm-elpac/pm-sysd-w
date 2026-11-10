//! Linux 平台 (pa) 实现.
//!
//! 仅用于方便测试 (Linux 上已有 systemd, pm-sysd-w 几乎无用).
//!
//! IPC 使用 当前目录 的 UNIX socket.

mod error;
#[cfg(test)]
mod test;

pub use error::PuError;

use std::path::PathBuf;

use tokio::net::{UnixListener, UnixStream};

use crate::pa::{BoxFuture, BoxStream, Pa, PaError, PaListener};

/// Linux 平台实现.
#[derive(Debug, Clone, Default)]
pub struct PaU;

impl Pa for PaU {
    fn name(&self) -> &'static str {
        "linux"
    }

    fn ipc_listen(&self, path: &str) -> Result<Box<dyn PaListener>, PaError> {
        // 直接 bind: 若路径已存在 (同名实例 已运行), bind 会失败, 即 实例唯一性检测.
        let inner = UnixListener::bind(path)
            .map_err(|e| PaError::Ipc(format!("bind `{}`: {}", path, e)))?;
        Ok(Box::new(UListener {
            path: PathBuf::from(path),
            inner,
        }))
    }

    fn ipc_connect(&self, path: &str) -> BoxFuture<'static, Result<BoxStream, PaError>> {
        let path = path.to_string();
        Box::pin(async move {
            let s = UnixStream::connect(&path)
                .await
                .map_err(|e| PaError::Ipc(format!("connect `{}`: {}", path, e)))?;
            Ok(Box::new(s) as BoxStream)
        })
    }
}

/// Linux UNIX socket 监听器.
///
/// 被 Drop 时 删除 socket 文件 (pm-sysd-init 正常退出 与 测试 结束时 自动清理).
struct UListener {
    /// socket 文件 路径.
    path: PathBuf,
    inner: UnixListener,
}

impl PaListener for UListener {
    fn accept<'a>(&'a mut self) -> BoxFuture<'a, Result<BoxStream, PaError>> {
        let fut = self.inner.accept();
        Box::pin(async move {
            let (stream, _) = fut.await.map_err(|e| PaError::Ipc(e.to_string()))?;
            Ok(Box::new(stream) as BoxStream)
        })
    }
}

impl Drop for UListener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
