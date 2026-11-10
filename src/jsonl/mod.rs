//! init 与 ctl 之间的 JSONL 通信协议.
//!
//! 传输基于 JSONL (单行 JSON), 使用 `serde_json` 编解码.
//!
//! 握手消息格式 (外层信封 + `data`):
//!
//! ```json
//! {"pm-sysd-w":1,"data":{"version":"0.1.0-a1"}}
//! ```
//!
//! - `pm-sysd-w`: 传输协议格式版本 (信封结构版本), 当前固定为 1.
//! - `data.version`: pm-sysd-w 软件版本号 (字符串, semver), 用于版本检查.
//!
//! 握手后, 后续消息无需版本号. 后续消息为 [`Command`] (ctl -> init) 或
//! [`Response`] (init -> ctl), 二者 均为 结构化 数据.

mod error;
#[cfg(test)]
mod test;

pub use error::JsonlError;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// 传输协议格式版本 (信封结构版本).
pub const PROTOCOL_VERSION: u32 = 1;

/// pm-sysd-w 软件版本号 (来自 `Cargo.toml` 的 `version`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 握手消息 (外层信封).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    /// 传输协议格式版本, 固定为 [`PROTOCOL_VERSION`].
    #[serde(rename = "pm-sysd-w")]
    pub protocol: u32,
    /// 消息内容.
    pub data: HelloData,
}

/// 握手消息 `data` 内容.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloData {
    /// pm-sysd-w 软件版本号 (用于 init/ctl 版本检查).
    pub version: String,
}

impl Hello {
    /// 构造握手消息 (使用本 crate 的软件版本号 [`VERSION`]).
    pub fn new() -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            data: HelloData {
                version: VERSION.to_string(),
            },
        }
    }
}

/// ctl 发给 init 的 命令 (结构化).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Command {
    /// 列出 unit.
    ListUnits,
    /// 列出 timer (初版为空).
    ListTimers,
    /// 读取 日志 (类似 journalctl).
    Logs {
        /// 日志查询条件.
        query: LogQuery,
    },
    /// 查询 unit 是否 active.
    IsActive {
        /// unit 名称.
        unit: String,
    },
    /// 查询 unit 是否 failed.
    IsFailed {
        /// unit 名称.
        unit: String,
    },
    /// 查询 unit 状态.
    Status {
        /// unit 名称.
        unit: String,
    },
    /// 显示 unit 配置 (key=value).
    Show {
        /// unit 名称.
        unit: String,
    },
    /// 显示 unit 配置 (类 unit 文件).
    Cat {
        /// unit 名称.
        unit: String,
    },
    /// 列出依赖.
    ListDependencies {
        /// unit 名称.
        unit: String,
    },
    /// 启动.
    Start {
        /// unit 名称.
        unit: String,
    },
    /// 停止.
    Stop {
        /// unit 名称.
        unit: String,
    },
    /// reload.
    Reload {
        /// unit 名称.
        unit: String,
    },
    /// 重启.
    Restart {
        /// unit 名称.
        unit: String,
    },
    /// 仅 active 时 重启.
    TryRestart {
        /// unit 名称.
        unit: String,
    },
    /// 有 ExecReload 则 reload, 否则 restart.
    ReloadOrRestart {
        /// unit 名称.
        unit: String,
    },
    /// 仅 active 时 reload-or-restart.
    TryReloadOrRestart {
        /// unit 名称.
        unit: String,
    },
    /// 强制终止.
    Kill {
        /// unit 名称.
        unit: String,
    },
    /// 清理 状态.
    Clean {
        /// unit 名称.
        unit: String,
    },
    /// 清除 failed 状态 (unit 可省略).
    ResetFailed {
        /// unit 名称 (省略 则清除全部).
        unit: Option<String>,
    },
    /// enable.
    Enable {
        /// unit 名称.
        unit: String,
        /// 是否同时 start.
        #[serde(default)]
        now: bool,
    },
    /// disable.
    Disable {
        /// unit 名称.
        unit: String,
        /// 是否同时 stop.
        #[serde(default)]
        now: bool,
    },
    /// reenable.
    Reenable {
        /// unit 名称.
        unit: String,
    },
    /// preset (初版 TODO).
    Preset {
        /// unit 名称.
        unit: String,
    },
    /// 查询 enable 状态.
    IsEnabled {
        /// unit 名称.
        unit: String,
    },
    /// mask.
    Mask {
        /// unit 名称.
        unit: String,
    },
    /// unmask.
    Unmask {
        /// unit 名称.
        unit: String,
    },
    /// 读取 默认 target.
    GetDefault,
    /// 设置 默认 target.
    SetDefault {
        /// target 名称.
        target: String,
    },
    /// 显示 环境变量 (初版 TODO).
    ShowEnvironment,
    /// 设置 环境变量 (初版 TODO).
    SetEnvironment,
    /// 删除 环境变量 (初版 TODO).
    UnsetEnvironment,
    /// 导入 环境变量 (初版 TODO).
    ImportEnvironment,
    /// 重新加载 配置文件.
    DaemonReload,
    /// 重启 init 及所有 service.
    SoftReboot,
    /// 退出 init.
    Exit,
}

/// 日志查询条件 (用于 `Logs` 命令).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogQuery {
    /// 按 unit 过滤 (省略则不过滤).
    pub unit: Option<String>,
    /// 最多读取条数.
    #[serde(default)]
    pub limit: u32,
}

/// 一条日志记录 (用于 `Logs` 响应).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// 自增 id.
    pub id: i64,
    /// 时间戳 (UTC, ISO 8601).
    pub ts: String,
    /// 日志级别.
    pub level: String,
    /// 关联 unit 名 (可为空).
    pub unit: Option<String>,
    /// 日志内容.
    pub msg: String,
}

/// unit 的 运行状态 (用于 `list-units`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitInfo {
    /// unit 名称.
    pub name: String,
    /// 描述.
    pub description: String,
    /// 激活状态.
    pub active: ActiveState,
    /// 子状态.
    pub sub: SubState,
}

/// unit 的 详细状态 (用于 `status`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitStatus {
    /// unit 名称.
    pub name: String,
    /// 描述.
    pub description: String,
    /// 是否 enable.
    pub enabled: bool,
    /// 是否 mask.
    pub masked: bool,
    /// 激活状态.
    pub active: ActiveState,
    /// 子状态.
    pub sub: SubState,
    /// 进程 pid (运行中).
    pub pid: Option<u32>,
    /// 自动重启次数.
    pub restart_count: u32,
    /// 最后一次退出状态码.
    pub exit_status: Option<i32>,
}

/// unit 配置 (用于 `show` / `cat`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitConfig {
    /// unit 名称.
    pub name: String,
    /// 描述.
    pub description: String,
    /// 启动类型 (配置关键字).
    pub service_type: String,
    /// 启动命令.
    pub exec_start: String,
    /// 停止命令.
    pub exec_stop: Option<String>,
    /// reload 命令.
    pub exec_reload: Option<String>,
    /// 重启策略 (配置关键字).
    pub restart: String,
    /// 重启间隔 (秒).
    pub restart_sec: u32,
    /// 所属 target.
    pub wanted_by: Option<String>,
}

/// unit 激活状态.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActiveState {
    /// 运行中 (或 remain-after-exit).
    Active,
    /// 未运行.
    Inactive,
    /// 失败.
    Failed,
}

/// unit 子状态.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SubState {
    /// 运行中.
    Running,
    /// 已退出 (remain-after-exit).
    Exited,
    /// 未启动.
    Dead,
    /// 失败.
    Failed,
}

/// enable 状态.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnableState {
    /// 已 enable.
    Enabled,
    /// 未 enable.
    Disabled,
    /// 已 mask.
    Masked,
}

/// timer 信息 (初版为空, 保留).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimerInfo {
    /// timer 名称.
    pub name: String,
}

/// init 回复给 ctl 的 响应 (结构化).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "kebab-case")]
pub enum Response {
    /// 操作成功 (无需额外数据).
    Ok,
    /// 操作失败 (携带 错误消息).
    Err {
        /// 错误消息 (尽力而为).
        message: String,
    },
    /// `list-units` 结果.
    ListUnits {
        /// unit 列表.
        units: Vec<UnitInfo>,
    },
    /// `status` 结果.
    Status {
        /// unit 状态.
        status: UnitStatus,
    },
    /// `show` / `cat` 结果.
    Config {
        /// unit 配置.
        config: UnitConfig,
    },
    /// `is-active` / `is-failed` 结果.
    Bool {
        /// 布尔值.
        value: bool,
    },
    /// `is-enabled` 结果.
    EnableState {
        /// enable 状态.
        state: EnableState,
    },
    /// `get-default` 结果.
    DefaultTarget {
        /// 默认 target.
        target: String,
    },
    /// `list-dependencies` 结果.
    Dependencies {
        /// 依赖 列表.
        deps: Vec<String>,
    },
    /// `list-timers` 结果.
    Timers {
        /// timer 列表.
        timers: Vec<TimerInfo>,
    },
    /// `logs` 结果.
    Logs {
        /// 日志列表.
        logs: Vec<LogEntry>,
    },
}

/// 握手后 的 消息 (命令 或 响应).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Frame {
    /// 命令 (ctl -> init).
    Command(Command),
    /// 响应 (init -> ctl).
    Response(Response),
}

/// 把握手消息编码为 单行 JSON.
pub fn encode_hello(h: &Hello) -> Result<String, JsonlError> {
    serde_json::to_string(h).map_err(|e| JsonlError::Serde(e.to_string()))
}

/// 从 单行 JSON 解析握手消息.
pub fn decode_hello(line: &str) -> Result<Hello, JsonlError> {
    serde_json::from_str(line).map_err(|e| JsonlError::Serde(e.to_string()))
}

/// 检查对方握手消息 与 本地 是否兼容 (协议版本 与 软件版本 一致).
pub fn check_version(h: &Hello) -> Result<(), JsonlError> {
    if h.protocol != PROTOCOL_VERSION {
        return Err(JsonlError::Version(format!(
            "protocol version `{}` != `{}`",
            h.protocol, PROTOCOL_VERSION
        )));
    }
    if h.data.version != VERSION {
        return Err(JsonlError::Version(format!(
            "software version `{}` != `{}`",
            h.data.version, VERSION
        )));
    }
    Ok(())
}

/// 把消息编码为 单行 JSON.
pub fn encode_frame(f: &Frame) -> Result<String, JsonlError> {
    serde_json::to_string(f).map_err(|e| JsonlError::Serde(e.to_string()))
}

/// 从 单行 JSON 解析消息.
pub fn decode_frame(line: &str) -> Result<Frame, JsonlError> {
    serde_json::from_str(line).map_err(|e| JsonlError::Serde(e.to_string()))
}

/// 发送 一行 JSON (末尾追加换行).
async fn send_line<W: AsyncWrite + Unpin>(w: &mut W, line: &str) -> Result<(), JsonlError> {
    w.write_all(line.as_bytes())
        .await
        .map_err(|e| JsonlError::Io(e.to_string()))?;
    w.write_all(b"\n")
        .await
        .map_err(|e| JsonlError::Io(e.to_string()))?;
    w.flush().await.map_err(|e| JsonlError::Io(e.to_string()))?;
    Ok(())
}

/// 读取 一行 JSON (去掉末尾换行).
async fn recv_line<R: AsyncRead + Unpin>(r: &mut R) -> Result<String, JsonlError> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = r
            .read(&mut byte)
            .await
            .map_err(|e| JsonlError::Io(e.to_string()))?;
        if n == 0 {
            if buf.is_empty() {
                return Err(JsonlError::Eof);
            }
            break;
        }
        if byte[0] == b'\n' {
            break;
        }
        buf.push(byte[0]);
    }
    while buf.last() == Some(&b'\r') {
        buf.pop();
    }
    String::from_utf8(buf).map_err(|e| JsonlError::Bad(format!("invalid utf-8: {}", e)))
}

/// 发送 握手消息.
pub async fn send_hello<W: AsyncWrite + Unpin>(w: &mut W, h: &Hello) -> Result<(), JsonlError> {
    send_line(w, &encode_hello(h)?).await
}

/// 接收 握手消息.
pub async fn recv_hello<R: AsyncRead + Unpin>(r: &mut R) -> Result<Hello, JsonlError> {
    let line = recv_line(r).await?;
    decode_hello(&line)
}

/// 发送 消息 (命令 / 响应).
pub async fn send_frame<W: AsyncWrite + Unpin>(w: &mut W, f: &Frame) -> Result<(), JsonlError> {
    send_line(w, &encode_frame(f)?).await
}

/// 接收 消息 (命令 / 响应).
pub async fn recv_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<Frame, JsonlError> {
    let line = recv_line(r).await?;
    decode_frame(&line)
}
