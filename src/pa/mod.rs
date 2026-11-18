//! 底层平台 (操作系统) 抽象/封装.
//!
//! Windows 实现见 `crate::p_w`, Linux 实现见 `crate::p_u` (仅用于测试).
//!
//! 本模块使用条件编译选择具体平台实现 (`p`).
//! 其余模块只依赖本模块, 不直接使用 `p_u` / `p_w`.
//!
//! 抽象两部分差异:
//!
//! - 进程 (process) 管理: 启动 / 停止 / 读取 stdout / stderr.
//! - IPC 连接: Windows 命名管道 / Linux UNIX socket.

use std::{
    env,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, ChildStderr, ChildStdout, Command},
    sync::mpsc,
    time::timeout,
};

#[cfg(target_os = "linux")]
use log::warn;
#[cfg(target_os = "linux")]
use tokio::signal::unix::{SignalKind, signal};

#[cfg(target_os = "windows")]
use tokio::signal::ctrl_c;

#[cfg(target_os = "linux")]
use crate::p_u as p;

#[cfg(target_os = "windows")]
use crate::p_w as p;

mod error;

#[cfg(test)]
mod test;

pub use error::PaError;

/// 流式文本解码器.
///
/// 连续输入多个字节缓冲区, 解码结果按字符边界返回; 未构成完整字符的字节
/// 保留在解码器内部, 下次输入继续解码.
///
/// - Linux: 固定按 UTF-8 解码.
/// - Windows: 先按 UTF-8 解码, 失败后切换到系统 ACP 模式; ACP 再失败则
///   返回错误并退出 ACP 模式.
pub struct TextDecoder {
    inner: p::TextDecoder,
}

impl TextDecoder {
    /// 创建新的解码器
    pub fn new() -> Self {
        Self {
            inner: p::TextDecoder::new(),
        }
    }

    /// 输入一个字节缓冲区, 返回已解码的字符串
    pub fn decode(&mut self, bytes: &[u8]) -> Result<String, PaError> {
        self.inner.decode(bytes).map_err(PaError::from)
    }

    /// 结束输入: 解码内部剩余字节
    pub fn finish(&mut self) -> Result<String, PaError> {
        self.inner.finish().map_err(PaError::from)
    }
}

/// 单条消息最大长度 (字节, 10MB), 超出报错并关闭连接
pub const MAX_MESSAGE_LEN: usize = 10 * 1024 * 1024;

/// 单条消息发送超时
pub const SEND_TIMEOUT: Duration = Duration::from_secs(1);

/// 单条消息接收超时 (从收到第一个字节到最后一个字节)
pub const RECV_TIMEOUT: Duration = Duration::from_secs(1);

/// 进程启动选项
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpawnOptions {
    /// Windows: 使用 CREATE_NO_WINDOW 标志启动 (console 程序不显示黑窗口)
    ///
    /// 仅 Windows 平台有效, 其它平台忽略
    pub create_no_window: bool,
}

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

/// IPC 服务器 (命名管道 / UNIX socket), pm-sysd-init 使用.
///
/// 被 Drop 时释放底层监听 (Linux 上会删除 socket 文件).
pub struct SysdServer {
    inner: p::Listener,
}

impl SysdServer {
    /// 绑定 IPC 监听 (同时用于实例唯一性检测).
    ///
    /// `acl` 指定权限级别; Linux 实现忽略 (UNIX socket 权限默认).
    pub fn bind(path: &str, acl: IpcAcl) -> Result<Self, PaError> {
        Ok(Self {
            inner: bind_listener(path, acl)?,
        })
    }

    /// 接受下一个连接
    pub async fn accept(&mut self) -> Result<SysdServerConn, PaError> {
        let stream = self.inner.accept().await.map_err(PaError::from)?;
        Ok(SysdServerConn {
            inner: BufReader::new(stream),
        })
    }
}

/// 服务器端已接受的 IPC 连接.
///
/// 负责单条消息的收/发 (含最大长度限制与超时).
pub struct SysdServerConn {
    inner: BufReader<p::ServerStream>,
}

impl SysdServerConn {
    /// 发送一条消息 (末尾自动追加换行)
    pub async fn send(&mut self, msg: String) -> Result<(), PaError> {
        send_line(self.inner.get_mut(), &msg).await
    }

    /// 接收一条消息 (去掉末尾换行)
    pub async fn recv(&mut self) -> Result<String, PaError> {
        recv_line(&mut self.inner).await
    }
}

/// IPC 客户端 (命名管道 / UNIX socket), pm-sysd-ctl / pm-sysd-log 使用.
pub struct SysdClient {
    inner: BufReader<p::ClientStream>,
}

impl SysdClient {
    /// 连接指定实例的 IPC 端点.
    ///
    /// 规则:
    ///
    /// - 先检查端点是否存在; 不存在立即返回 [`PaError::IpcNotFound`] (快速失败).
    /// - Windows `--system`: 优先连接主管道; 权限不足时回退到只读管道 `_ro`.
    pub async fn connect(instance: &Instance) -> Result<Self, PaError> {
        let transport = choose_transport(instance, ipc_exists, None)?;
        match Self::connect_transport(&transport).await {
            Ok(client) => Ok(client),
            Err(e) if matches!(&e, PaError::IpcPermission(_)) => {
                let ro = choose_transport(instance, ipc_exists, Some(e))?;
                Self::connect_transport(&ro).await
            }
            Err(e) => Err(e),
        }
    }

    /// 连接指定 transport (平台原始地址)
    async fn connect_transport(transport: &str) -> Result<Self, PaError> {
        let stream = p::connect(transport).await.map_err(PaError::from)?;
        Ok(Self {
            inner: BufReader::new(stream),
        })
    }

    /// 发送一条消息 (末尾自动追加换行)
    pub async fn send(&mut self, msg: String) -> Result<(), PaError> {
        send_line(self.inner.get_mut(), &msg).await
    }

    /// 接收一条消息 (去掉末尾换行)
    pub async fn recv(&mut self) -> Result<String, PaError> {
        recv_line(&mut self.inner).await
    }
}

/// 子进程封装.
///
/// stdout / stderr 以管道形式接管 (供上层读取并写入日志).
pub struct Process {
    child: Child,
}

impl Process {
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

/// 检查 IPC 端点是否存在 (不建立连接)
pub fn ipc_exists(path: &str) -> bool {
    p::exists(path)
}

/// 重启自身 (soft-reboot): 使用相同命令行参数重新运行当前程序.
pub fn soft_reboot(args: &[String]) -> Result<(), PaError> {
    p::soft_reboot(args).map_err(PaError::from)
}

/// 启动外部优雅退出信号监听任务.
///
/// Linux: SIGTERM.
/// Windows: Ctrl-C (命令行) 或 (服务模式) 服务停止请求.
///
/// 收到信号后向 `tx` 发送一次 `()`.
pub fn spawn_signal_watch(tx: mpsc::Sender<()>) {
    #[cfg(unix)]
    {
        tokio::spawn(async move {
            match signal(SignalKind::terminate()) {
                Ok(mut sig) => {
                    sig.recv().await;
                    tx.send(()).await.expect("signal watch receiver dropped");
                }
                Err(e) => {
                    warn!("install SIGTERM handler: {}", e);
                }
            }
        });
    }

    #[cfg(windows)]
    {
        let service = p::service::is_service();
        let stop = p::service::stop_event();

        tokio::spawn(async move {
            if service {
                // Windows 服务模式: 等待 SCM 停止请求 (无控制台, 不监听 Ctrl+C).
                stop.wait().await;
            } else if ctrl_c().await.is_err() {
                // Ctrl+C 监听失败 (例如无控制台): 保持等待, 不发送退出信号.
                return;
            }
            tx.send(()).await.expect("signal watch receiver dropped");
        });
    }
}

/// 当前主机名 (日志显示用)
pub fn hostname() -> String {
    #[cfg(target_os = "linux")]
    {
        env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string())
    }
    #[cfg(windows)]
    {
        env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".to_string())
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        "localhost".to_string()
    }
}

/// 相对路径转绝对路径 (基于 `base`)
pub fn resolve_path(base: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
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
async fn send_line<W: AsyncWrite + Unpin>(w: &mut W, line: &str) -> Result<(), PaError> {
    let mut data = Vec::with_capacity(line.len() + 1);
    data.extend_from_slice(line.as_bytes());
    data.push(b'\n');

    match timeout(SEND_TIMEOUT, w.write_all(&data)).await {
        Ok(r) => r.map_err(|e| PaError::Io(e.to_string()))?,
        Err(_) => {
            return Err(PaError::Timeout(format!(
                "send exceeded {} ms",
                SEND_TIMEOUT.as_millis()
            )));
        }
    }

    match timeout(SEND_TIMEOUT, w.flush()).await {
        Ok(r) => r.map_err(|e| PaError::Io(e.to_string()))?,
        Err(_) => {
            return Err(PaError::Timeout(format!(
                "flush exceeded {} ms",
                SEND_TIMEOUT.as_millis()
            )));
        }
    }
    Ok(())
}

/// 读取一行消息 (去掉末尾换行), 带最大长度限制与超时.
///
/// `RECV_TIMEOUT` 只约束从收到本条消息第一个字节到最后一个字节的过程,
/// 不约束消息到来之前的等待. 对方完全不发消息时, 会一直等待.
async fn recv_line<R: AsyncBufRead + Unpin>(r: &mut R) -> Result<String, PaError> {
    // 等待消息第一个字节 (消息到来). 此处不加超时.
    {
        let first = r.fill_buf().await.map_err(|e| PaError::Io(e.to_string()))?;
        if first.is_empty() {
            return Err(PaError::Eof);
        }
    }

    // 从第一个字节起, 读取本条消息的剩余部分, 并应用 RECV_TIMEOUT.
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
                    if buf.len() > MAX_MESSAGE_LEN {
                        return Err(PaError::TooLong(format!(
                            "message exceeds {} bytes",
                            MAX_MESSAGE_LEN
                        )));
                    }
                    r.consume(pos + 1);
                    break;
                }
                None => {
                    buf.extend_from_slice(available);
                    let n = available.len();
                    r.consume(n);
                    if buf.len() > MAX_MESSAGE_LEN {
                        return Err(PaError::TooLong(format!(
                            "message exceeds {} bytes",
                            MAX_MESSAGE_LEN
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

    match timeout(RECV_TIMEOUT, inner).await {
        Ok(res) => res,
        Err(_) => Err(PaError::Timeout(format!(
            "receive exceeded {} ms",
            RECV_TIMEOUT.as_millis()
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

#[cfg(target_os = "linux")]
impl From<crate::p_u::PuError> for PaError {
    fn from(e: crate::p_u::PuError) -> Self {
        use crate::p_u::PuError as E;
        match e {
            E::Ipc(m) => PaError::Ipc(m),
            E::NotFound(m) => PaError::IpcNotFound(m),
            E::Permission(m) => PaError::IpcPermission(m),
            E::Msg(m) => PaError::Msg(m),
        }
    }
}

#[cfg(windows)]
impl From<crate::p_w::PwError> for PaError {
    fn from(e: crate::p_w::PwError) -> Self {
        use crate::p_w::PwError as E;
        match e {
            E::Ipc(m) => PaError::Ipc(m),
            E::NotFound(m) => PaError::IpcNotFound(m),
            E::Permission(m) => PaError::IpcPermission(m),
            E::Msg(m) => PaError::Msg(m),
        }
    }
}
