//! Linux 平台 (pa) 实现.
//!
//! 仅用于方便测试 (Linux 上已有 systemd, pm-sysd-w 几乎无用).
//!
//! IPC 使用 UNIX socket. 本模块 只包含 平台相关 代码, 不做 抽象/封装.

use std::{
    env, fs, io,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

use tokio::net::{UnixListener, UnixStream};

mod error;

#[cfg(test)]
mod test;

pub use error::PuError;

/// 客户端 连接流 (Linux 上 与服务端 同类型).
pub type ClientStream = UnixStream;

/// 服务端 连接流.
pub type ServerStream = UnixStream;

/// UNIX socket 监听器.
///
/// 被 Drop 时删除 socket 文件 (pm-sysd-init 正常退出与测试结束时自动清理).
pub struct Listener {
    /// socket 文件 路径.
    path: PathBuf,
    inner: UnixListener,
}

impl Listener {
    /// 绑定 UNIX socket. 路径已存在时 失败 (实例唯一性检测).
    pub fn bind(path: &str) -> Result<Self, PuError> {
        let inner = UnixListener::bind(path)
            .map_err(|e| PuError::Ipc(format!("bind `{}`: {}", path, e)))?;
        Ok(Self {
            path: PathBuf::from(path),
            inner,
        })
    }

    /// 接受 下一个连接.
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

/// 连接 UNIX socket.
pub async fn connect(path: &str) -> Result<ClientStream, PuError> {
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

/// 检查 UNIX socket 文件 是否存在.
pub fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

/// 重启自身 (soft-reboot): `exec` 替换当前进程, 失败时返回错误.
pub fn soft_reboot(args: &[String]) -> Result<(), PuError> {
    let exe = env::current_exe().map_err(|e| PuError::Msg(format!("current_exe: {}", e)))?;
    let e = Command::new(&exe).args(args).exec();
    Err(PuError::Msg(format!("soft-reboot exec: {}", e)))
}
