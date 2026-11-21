//! IPC 连接: Windows 命名管道 / Linux UNIX socket.

use std::{env, time::Duration};

use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    time::timeout,
};

use crate::conf::limit::{IpcLimit, SysdClientLimit};

#[cfg(target_os = "linux")]
use crate::p_u as p;

#[cfg(target_os = "windows")]
use crate::p_w as p;

use super::PaError;

/// IPC 权限级别 (用于命名管道 ACL / 实例权限控制)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcAcl {
    /// 系统级主管道: 仅管理员 (以及 SYSTEM)
    System,
    /// 系统级只读管道: 本机普通用户只读查询
    SystemReadOnly,
    /// 用户级 / 自定义实例: 仅当前用户
    CurrentUser,
}

/// IPC 监听器 (命名管道 / UNIX socket), pm-sysd-init 使用.
///
/// 被 Drop 时释放底层监听 (Linux 上会删除 socket 文件).
pub struct SysdListener {
    inner: p::Listener,
    ipc: IpcLimit,
}

impl SysdListener {
    /// 绑定 IPC 监听 (同时用于实例唯一性检测).
    ///
    /// `acl` 指定权限级别; Linux 实现忽略 (UNIX socket 权限默认).
    pub fn bind(path: &str, acl: IpcAcl, ipc: IpcLimit) -> Result<Self, PaError> {
        Ok(Self {
            inner: bind_listener(path, acl)?,
            ipc,
        })
    }

    /// 接受下一个连接
    pub async fn accept(&mut self) -> Result<SysdConn, PaError> {
        let stream = self.inner.accept().await.map_err(PaError::from)?;
        Ok(SysdConn {
            inner: BufReader::new(stream),
            ipc: self.ipc,
        })
    }
}

/// 服务器端已接受的 IPC 连接.
///
/// 负责单条消息的收/发 (含最大长度限制与超时).
pub struct SysdConn {
    inner: BufReader<p::ServerStream>,
    ipc: IpcLimit,
}

impl SysdConn {
    /// 发送一条消息 (末尾自动追加换行)
    pub async fn send(&mut self, msg: String) -> Result<(), PaError> {
        send_line(self.inner.get_mut(), &msg, self.ipc.send_timeout).await
    }

    /// 接收一条消息 (去掉末尾换行)
    pub async fn recv(&mut self) -> Result<String, PaError> {
        recv_line(&mut self.inner, self.ipc.recv_timeout, self.ipc.m_byte).await
    }

    /// 等待下一条消息到达 (出现第一个字节), 最多等待 `wait`.
    ///
    /// 只约束消息首字节之前的等待; 首字节之后由 [`Self::recv`] 读取完整消息,
    /// 其中已包含 [`IpcLimit::recv_timeout`].
    pub async fn wait_message(&mut self, wait: Duration) -> Result<(), PaError> {
        match timeout(wait, self.inner.fill_buf()).await {
            Ok(r) => {
                let buf = r.map_err(|e| PaError::Io(e.to_string()))?;
                if buf.is_empty() {
                    Err(PaError::Eof)
                } else {
                    Ok(())
                }
            }
            Err(_) => Err(PaError::Timeout(format!(
                "wait message exceeded {} ms",
                wait.as_millis()
            ))),
        }
    }
}

/// IPC 客户端 (命名管道 / UNIX socket), pm-sysd-ctl / pm-sysd-log 使用.
pub struct SysdClient {
    inner: BufReader<p::ClientStream>,
    ipc: IpcLimit,
}

impl SysdClient {
    /// 连接指定实例的 IPC 端点.
    ///
    /// 规则:
    ///
    /// - 先检查端点是否存在; 不存在立即返回 [`PaError::IpcNotFound`] (快速失败).
    /// - Windows `--system`: 优先连接主管道; 权限不足时回退到只读管道 `_ro`.
    pub async fn connect(instance: &Instance, limit: &SysdClientLimit) -> Result<Self, PaError> {
        let transport = choose_transport(instance, ipc_exists, None)?;
        match Self::connect_transport(&transport, limit).await {
            Ok(client) => Ok(client),
            Err(e) if matches!(&e, PaError::IpcPermission(_)) => {
                let ro = choose_transport(instance, ipc_exists, Some(e))?;
                Self::connect_transport(&ro, limit).await
            }
            Err(e) => Err(e),
        }
    }

    /// 连接指定 transport (平台原始地址)
    async fn connect_transport(transport: &str, limit: &SysdClientLimit) -> Result<Self, PaError> {
        let stream = p::connect(
            transport,
            limit.ipc_connect_timeout,
            limit.ipc_connect_retry_interval,
        )
        .await
        .map_err(PaError::from)?;
        Ok(Self {
            inner: BufReader::new(stream),
            ipc: limit.ipc(),
        })
    }

    /// 发送一条消息 (末尾自动追加换行)
    pub async fn send(&mut self, msg: String) -> Result<(), PaError> {
        send_line(self.inner.get_mut(), &msg, self.ipc.send_timeout).await
    }

    /// 接收一条消息 (去掉末尾换行)
    pub async fn recv(&mut self) -> Result<String, PaError> {
        recv_line(&mut self.inner, self.ipc.recv_timeout, self.ipc.m_byte).await
    }
}

/// 检查 IPC 端点是否存在 (不建立连接)
pub fn ipc_exists(path: &str) -> bool {
    p::exists(path)
}

/// pm-sysd-w 实例 (决定 IPC 地址)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instance {
    /// 系统级实例 (`--system`)
    System,
    /// 用户级实例 (`--user`)
    User,
    /// 自定义实例 (`--machine=XXX`)
    Machine(String),
}

impl Instance {
    /// 实例名称 (不含平台路径前缀)
    pub fn name(&self) -> String {
        match self {
            Self::System => "pm_sysd_w_system_default".to_string(),
            Self::User => format!("pm_sysd_w_user_{}", current_user()),
            Self::Machine(m) => format!("pm_sysd_w_m_{}", m),
        }
    }

    /// 实例类型 (对应 `--system` / `--user` / `--machine`).
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Machine(_) => "machine",
        }
    }

    /// 实例显示名称.
    ///
    /// `--user` 为用户 名, `--machine` 为指定的名称, `--system` 为空.
    pub fn display_name(&self) -> String {
        match self {
            Self::System => String::new(),
            Self::User => current_user(),
            Self::Machine(m) => m.clone(),
        }
    }

    /// IPC 传输地址.
    ///
    /// - Linux: 当前目录的 UNIX socket 文件.
    /// - Windows: 命名管道名称.
    pub fn transport(&self) -> String {
        let name = self.name();
        #[cfg(target_os = "linux")]
        {
            format!("./{}.sock", name)
        }
        #[cfg(windows)]
        {
            name
        }
    }

    /// 只读 IPC 传输地址 (仅 Windows 的 `--system` 实例).
    ///
    /// `--system` 实例在 Windows 上除了主管道, 还会创建 `_ro` 只读命名管道;
    /// 普通用户连接主管道权限不足时, 可回退到只读管道.
    pub fn ro_transport(&self) -> Option<String> {
        #[cfg(windows)]
        {
            if matches!(self, Self::System) {
                Some(format!("{}_ro", self.transport()))
            } else {
                None
            }
        }
        #[cfg(not(windows))]
        {
            // 非 Windows 无只读管道, self 未使用; 标记为已用避免 unused 警告.
            let _ = self;
            None
        }
    }
}

/// 选择实际连接的 transport (纯决策, 无 IO).
///
/// `main_err` 为连接主 transport 的结果 (成功时 `None`). 用于在
/// Windows `--system` 权限不足时回退到只读管道; 也便于单元测试.
pub fn choose_transport(
    instance: &Instance,
    exists: impl Fn(&str) -> bool,
    main_err: Option<PaError>,
) -> Result<String, PaError> {
    let transport = instance.transport();
    if !exists(&transport) {
        return Err(PaError::IpcNotFound(format!(
            "instance `{}` is not running (`{}` not found)",
            instance.name(),
            transport
        )));
    }

    match main_err {
        None => Ok(transport),
        Some(e) if matches!(&e, PaError::IpcPermission(_)) => {
            if let Some(ro) = instance.ro_transport() {
                if exists(&ro) { Ok(ro) } else { Err(e) }
            } else {
                Err(e)
            }
        }
        Some(e) => Err(e),
    }
}

/// 平台相关: 绑定 IPC 监听
#[cfg(target_os = "linux")]
fn bind_listener(path: &str, acl: IpcAcl) -> Result<p::Listener, PaError> {
    // Linux 的 IPC 无需 ACL, 参数 acl 未使用; 标记为已用避免 unused 警告.
    let _ = acl;
    p::Listener::bind(path).map_err(PaError::from)
}

/// 平台相关: 绑定 IPC 监听
#[cfg(windows)]
fn bind_listener(path: &str, acl: IpcAcl) -> Result<p::Listener, PaError> {
    p::Listener::bind(path, acl.into()).map_err(PaError::from)
}

/// 发送一行消息 (末尾追加换行), 带超时
async fn send_line<W: AsyncWrite + Unpin>(
    w: &mut W,
    line: &str,
    send_timeout: Duration,
) -> Result<(), PaError> {
    let mut data = Vec::with_capacity(line.len() + 1);
    data.extend_from_slice(line.as_bytes());
    data.push(b'\n');

    match timeout(send_timeout, w.write_all(&data)).await {
        Ok(r) => r.map_err(|e| PaError::Io(e.to_string()))?,
        Err(_) => {
            return Err(PaError::Timeout(format!(
                "send exceeded {} ms",
                send_timeout.as_millis()
            )));
        }
    }

    match timeout(send_timeout, w.flush()).await {
        Ok(r) => r.map_err(|e| PaError::Io(e.to_string()))?,
        Err(_) => {
            return Err(PaError::Timeout(format!(
                "flush exceeded {} ms",
                send_timeout.as_millis()
            )));
        }
    }
    Ok(())
}

/// 读取一行消息 (去掉末尾换行), 带最大长度限制与超时.
///
/// `recv_timeout` 只约束从收到本条消息第一个字节到最后一个字节的过程,
/// 不约束消息到来之前的等待. 对方完全不发消息时, 会一直等待.
async fn recv_line<R: AsyncBufRead + Unpin>(
    r: &mut R,
    recv_timeout: Duration,
    m_byte: usize,
) -> Result<String, PaError> {
    // 等待消息第一个字节 (消息到来). 此处不加超时.
    {
        let first = r.fill_buf().await.map_err(|e| PaError::Io(e.to_string()))?;
        if first.is_empty() {
            return Err(PaError::Eof);
        }
    }

    // 从第一个字节起, 读取本条消息的剩余部分, 并应用 `recv_timeout`.
    let inner = async {
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let available = r.fill_buf().await.map_err(|e| PaError::Io(e.to_string()))?;
            if available.is_empty() {
                if buf.is_empty() {
                    return Err(PaError::Eof);
                }
                break;
            }

            match available.iter().position(|&b| b == b'\n') {
                Some(pos) => {
                    buf.extend_from_slice(&available[..pos]);
                    if buf.len() > m_byte {
                        return Err(PaError::TooLong(format!(
                            "message exceeds {} bytes",
                            m_byte
                        )));
                    }
                    r.consume(pos + 1);
                    break;
                }
                None => {
                    buf.extend_from_slice(available);
                    let n = available.len();
                    r.consume(n);
                    if buf.len() > m_byte {
                        return Err(PaError::TooLong(format!(
                            "message exceeds {} bytes",
                            m_byte
                        )));
                    }
                }
            }
        }

        while buf.last() == Some(&b'\r') {
            buf.pop();
        }
        String::from_utf8(buf).map_err(|e| PaError::Msg(format!("invalid utf-8: {}", e)))
    };

    match timeout(recv_timeout, inner).await {
        Ok(res) => res,
        Err(_) => Err(PaError::Timeout(format!(
            "receive exceeded {} ms",
            recv_timeout.as_millis()
        ))),
    }
}

/// 当前用户名 (实例命名用)
#[cfg(target_os = "linux")]
fn current_user() -> String {
    env::var("USER").unwrap_or_else(|_| "unknown".to_string())
}

/// 当前用户名 (实例命名用)
#[cfg(windows)]
fn current_user() -> String {
    env::var("USERNAME").unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod test;
