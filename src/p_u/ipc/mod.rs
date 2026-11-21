//! p_u::ipc: Linux UNIX socket IPC.

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use tokio::net::{UnixListener, UnixStream};

use super::PuError;

/// 客户端连接流 (Linux 上与服务端同类型)
pub type ClientStream = UnixStream;

/// 服务端连接流
pub type ServerStream = UnixStream;

/// UNIX socket 监听器.
///
/// 被 Drop 时删除 socket 文件 (pm-sysd-init 正常退出与测试结束时自动清理).
pub struct Listener {
    /// socket 文件路径.
    path: PathBuf,
    inner: UnixListener,
}

impl Listener {
    /// 绑定 UNIX socket. 路径已存在时失败 (实例唯一性检测).
    pub fn bind(path: &str) -> Result<Self, PuError> {
        let inner = UnixListener::bind(path)
            .map_err(|e| PuError::Ipc(format!("bind `{}`: {}", path, e)))?;
        Ok(Self {
            path: PathBuf::from(path),
            inner,
        })
    }

    /// 接受下一个连接
    pub async fn accept(&mut self) -> Result<ServerStream, PuError> {
        let (stream, _) = self
            .inner
            .accept()
            .await
            .map_err(|e| PuError::Ipc(e.to_string()))?;
        Ok(stream)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // 删除 UNIX socket 文件 失败时 无法向上传播 (Drop 无返回值), 尽力而为, 忽略.
        let _ = fs::remove_file(&self.path);
    }
}

/// 连接 UNIX socket
pub async fn connect(
    path: &str,
    _connect_timeout: Duration,
    _retry_interval: Duration,
) -> Result<ClientStream, PuError> {
    UnixStream::connect(path).await.map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            PuError::NotFound(format!("connect `{}`: {}", path, e))
        } else if e.kind() == io::ErrorKind::PermissionDenied {
            PuError::Permission(format!("connect `{}`: {}", path, e))
        } else {
            PuError::Ipc(format!("connect `{}`: {}", path, e))
        }
    })
}

/// 检查 UNIX socket 文件是否存在
pub fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

#[cfg(test)]
mod test;
