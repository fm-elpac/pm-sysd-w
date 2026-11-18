//! Linux 平台 (pa) 实现.
//!
//! 仅用于方便测试 (Linux 上已有 systemd, pm-sysd-w 几乎无用).
//!
//! IPC 使用 UNIX socket. 本模块只包含平台相关代码, 不做抽象/封装.

use std::{
    env, fs, io,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    str,
};

use tokio::net::{UnixListener, UnixStream};

mod error;

#[cfg(test)]
mod test;

pub use error::PuError;

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

/// 流式 UTF-8 解码器.
///
/// 连续输入多个字节缓冲区, 解码结果按字符边界返回; 未构成完整字符的字节
/// 保留在解码器内部, 下次输入继续解码.
pub struct TextDecoder {
    /// 待解码字节 (可能为空)
    pending: Vec<u8>,
}

impl TextDecoder {
    /// 创建新的 UTF-8 解码器
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// 输入一个字节缓冲区, 返回已解码的 UTF-8 字符串.
    ///
    /// 若缓冲区以不完整的多字节字符结尾, 只返回前面完整字符, 剩余字节留到
    /// 下次. 若缓冲区包含非法 UTF-8, 返回错误并丢弃所有数据.
    pub fn decode(&mut self, bytes: &[u8]) -> Result<String, PuError> {
        if bytes.is_empty() {
            return Ok(String::new());
        }

        self.pending.extend_from_slice(bytes);
        let data = std::mem::take(&mut self.pending);
        match str::from_utf8(&data) {
            Ok(s) => Ok(s.to_string()),
            Err(e) => {
                let valid = e.valid_up_to();
                if valid > 0 {
                    let s = str::from_utf8(&data[..valid])
                        .expect("valid_up_to 保证前缀是合法 UTF-8")
                        .to_string();
                    self.pending.extend_from_slice(&data[valid..]);
                    Ok(s)
                } else if e.error_len().is_some() {
                    // 解码失败: 返回错误前丢弃所有数据, 避免内部缓冲区膨胀
                    self.pending.clear();
                    Err(PuError::Msg(format!("invalid utf-8: {}", e)))
                } else {
                    // 整个缓冲区是不完整字符的前缀, 全部保留待后续输入
                    self.pending = data;
                    Ok(String::new())
                }
            }
        }
    }

    /// 结束输入: 解码内部剩余字节.
    ///
    /// 剩余字节为空时返回空字符串. 否则必须是完整合法 UTF-8, 失败返回错误.
    pub fn finish(&mut self) -> Result<String, PuError> {
        let data = std::mem::take(&mut self.pending);
        if data.is_empty() {
            return Ok(String::new());
        }
        match str::from_utf8(&data) {
            Ok(s) => Ok(s.to_string()),
            Err(e) => Err(PuError::Msg(format!("invalid utf-8 at end: {}", e))),
        }
    }
}

/// 连接 UNIX socket
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

/// 检查 UNIX socket 文件是否存在
pub fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

/// 重启自身 (soft-reboot): `exec` 替换当前进程, 失败时返回错误
pub fn soft_reboot(args: &[String]) -> Result<(), PuError> {
    let exe = env::current_exe().map_err(|e| PuError::Msg(format!("current_exe: {}", e)))?;
    let e = Command::new(&exe).args(args).exec();
    Err(PuError::Msg(format!("soft-reboot exec: {}", e)))
}
