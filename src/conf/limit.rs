//! 防御限制常量与运行时限制配置.
//!
//! 集中定义所有防御限制 (内存目标见 `doc/dev/limit.md`), 覆盖配置解析, IPC,
//! 日志, 数据库等.
//!
//! ## 命名约定
//!
//! - 各常量默认表示 "最大" 限制, 名称省略 `MAX`. 表示 "最小" 限制时, 名称加上 `MIN`.
//! - 名称结尾标注单位: 字节用 `_BYTE`, 毫秒用 `_MS`, 分钟用 `_M`, 个数用 `_N`.
//! - 各常量统一以 `SYSD_LIMIT_` 开头. 唯一例外是 [`SYSD_CONF_LIMIT_TOTAL_BYTE`].
//! - 各常量默认供 pm-sysd-init 使用. 供 pm-sysd-ctl / pm-sysd-log 使用的常量,
//!   会在各自注释中注明.
//!
//! ## 运行时配置
//!
//! 常量只是默认值. [`SysdLimit`] / [`SysdClientLimit`] 由各使用模块自己持有,
//! 通过 [`SysdLimit::from_env`] / [`SysdClientLimit::from_env`] 在程序启动 /
//! 初始化时读取一次同名环境变量覆盖. 之后这些值不再变化.

use std::{env, str::FromStr, time::Duration};

/// 所有 toml 文件总大小上限 (字节).
///
/// 这是唯一不以 `SYSD_LIMIT_` 开头的例外.
pub const SYSD_CONF_LIMIT_TOTAL_BYTE: usize = 10 * 1024 * 1024;

/// unit 名称最大长度 (utf-8 字节)
pub const SYSD_LIMIT_UNIT_NAME_BYTE: usize = 500;

/// 单个受支持 `env:` 键最大长度 (utf-8 字节)
pub const SYSD_LIMIT_UNIT_KEY_BYTE: usize = 500;

/// 单个受支持 `env:` 值最大长度 (utf-8 字节)
pub const SYSD_LIMIT_UNIT_VALUE_BYTE: usize = 20 * 1024;

/// 单个 unit 的全部受支持键值总长上限 (utf-8 字节)
pub const SYSD_LIMIT_UNIT_TOTAL_BYTE: usize = 500 * 1024;

/// 单条 IPC 消息最大长度 (字节, 10MB), 超出报错并关闭连接.
///
/// pm-sysd-init (服务端) 与 pm-sysd-ctl / pm-sysd-log (客户端) 共用.
pub const SYSD_LIMIT_IPC_M_BYTE: usize = 10 * 1024 * 1024;

/// 单条 IPC 消息发送超时.
///
/// pm-sysd-init (服务端) 与 pm-sysd-ctl / pm-sysd-log (客户端) 共用.
pub const SYSD_LIMIT_IPC_SEND_TIMEOUT_MS: Duration = Duration::from_millis(1000);

/// 单条 IPC 消息接收超时 (从收到第一个字节到最后一个字节).
///
/// pm-sysd-init (服务端) 与 pm-sysd-ctl / pm-sysd-log (客户端) 共用.
pub const SYSD_LIMIT_IPC_RECV_TIMEOUT_MS: Duration = Duration::from_millis(1000);

/// 主管道最大并发连接数
pub const SYSD_LIMIT_IPC_CONN_N: usize = 200;

/// 只读管道最大并发连接数
pub const SYSD_LIMIT_IPC_CONN_RO_N: usize = 50;

/// 握手超时: 连接建立后, 收到握手消息前的最长等待时间
pub const SYSD_LIMIT_IPC_HANDSHAKE_TIMEOUT_MS: Duration = Duration::from_millis(1000);

/// 空闲超时: 未等待 SdReq 响应时, 收到下一条请求前的最长等待时间
pub const SYSD_LIMIT_IPC_IDLE_TIMEOUT_MS: Duration = Duration::from_millis(1000);

/// 软超时: 加载进程运行超时, 仍未返回时, 输出警告, 并继续等待
pub const SYSD_LIMIT_RELOAD_SOFT_TIMEOUT_MS: Duration = Duration::from_millis(1000);

/// 硬超时: 加载进程运行超时后, 终止, 并返回错误
pub const SYSD_LIMIT_RELOAD_HARD_TIMEOUT_MS: Duration = Duration::from_millis(2000);

/// 单条日志最大长度 (字节), 超出由发送方拆分为多条
pub const SYSD_LIMIT_LOG_LINE_BYTE: usize = 500 * 1024;

/// 单次日志分页查询允许的最大条数 (硬限制).
///
/// 外部请求的一页条数超过此值时, 自动按此值截断.
pub const SYSD_LIMIT_LOG_PAGE_N: usize = 10_000;

/// 日志自动清理间隔 (固定: 每 60 分钟一次)
pub const SYSD_LIMIT_LOG_CLEAN_INTERVAL_M: Duration = Duration::from_secs(60 * 60);

/// 日志保留时长 (固定: 最近 4320 分钟, 即 72 小时)
pub const SYSD_LIMIT_LOG_RETENTION_M: Duration = Duration::from_secs(4320 * 60);

/// 丢弃日志警告的最小间隔 (速率限制)
pub const SYSD_LIMIT_WARN_INTERVAL_MIN_MS: Duration = Duration::from_millis(1000);

/// 待写入日志数据总大小上限 (字节), 由日志批发线程检查
pub const SYSD_LIMIT_LOG_PENDING_BYTE: usize = 10 * 1024 * 1024;

/// 日志批发线程内部缓冲区最大条数.
///
/// 默认取 10 万条: 压力测试显示从 1 万提到 10 万时, Linux 入库吞吐提升最明显,
/// 再继续放大收益递减且内存明显上升 (见 `doc/dev/log.md` PENDING_N 扫描).
pub const SYSD_LIMIT_LOG_PENDING_N: usize = 100_000;

/// 日志输入 MPSC 通道容量.
///
/// 默认取 1000: 压力测试中限速无损速率随容量提升, 1000 时最高,
/// 且几乎不增加峰值内存 (见 `doc/dev/log.md` 通道容量扫描).
pub const SYSD_LIMIT_LOG_CHANNEL_CAPACITY_N: usize = 1000;

/// 数据库读写线程的请求缓冲区大小 (条)
pub const SYSD_LIMIT_DB_CHANNEL_CAPACITY_N: usize = 1000;

/// 通用 MPSC 通道容量 (actor 邮箱 / Server 控制通道 / 日志查询通道等)
pub const SYSD_LIMIT_CHANNEL_CAPACITY_N: usize = 64;

/// 退出时等待启动任务返回结果的最长等待时间
pub const SYSD_LIMIT_STARTUP_WAIT_TIMEOUT_MS: Duration = Duration::from_millis(5000);

/// Windows 命名管道连接快速失败的总耗时上限.
///
/// pm-sysd-ctl / pm-sysd-log 使用 (Windows 客户端连接).
pub const SYSD_LIMIT_IPC_CONNECT_TIMEOUT_MS: Duration = Duration::from_millis(200);

/// Windows 命名管道连接遇到 ERROR_PIPE_BUSY 时的重试间隔.
///
/// pm-sysd-ctl / pm-sysd-log 使用 (Windows 客户端连接).
pub const SYSD_LIMIT_IPC_CONNECT_RETRY_INTERVAL_MS: Duration = Duration::from_millis(20);

/// IPC 收发的单条消息限制 (供 [`pa::ipc`](crate::pa::ipc) 使用).
#[derive(Debug, Clone, Copy)]
pub struct IpcLimit {
    /// 单条消息最大长度 (字节)
    pub m_byte: usize,
    /// 发送超时
    pub send_timeout: Duration,
    /// 接收超时 (从收到第一个字节到最后一个字节)
    pub recv_timeout: Duration,
}

/// 所有 pm-sysd-init 使用的限制值.
///
/// 字段名省略 `SYSD_LIMIT_` 前缀, 默认值取自本模块同名常量.
#[derive(Debug, Clone)]
pub struct SysdLimit {
    /// 所有 toml 文件总大小上限 (字节)
    pub conf_total_byte: usize,
    /// unit 名称最大长度 (utf-8 字节)
    pub unit_name_byte: usize,
    /// 单个受支持 `env:` 键最大长度 (utf-8 字节)
    pub unit_key_byte: usize,
    /// 单个受支持 `env:` 值最大长度 (utf-8 字节)
    pub unit_value_byte: usize,
    /// 单个 unit 的全部受支持键值总长上限 (utf-8 字节)
    pub unit_total_byte: usize,
    /// 单条 IPC 消息最大长度 (字节)
    pub ipc_m_byte: usize,
    /// 单条 IPC 消息发送超时
    pub ipc_send_timeout: Duration,
    /// 单条 IPC 消息接收超时
    pub ipc_recv_timeout: Duration,
    /// 主管道最大并发连接数
    pub ipc_conn_n: usize,
    /// 只读管道最大并发连接数
    pub ipc_conn_ro_n: usize,
    /// 握手超时
    pub ipc_handshake_timeout: Duration,
    /// 空闲超时
    pub ipc_idle_timeout: Duration,
    /// 加载进程软超时
    pub reload_soft_timeout: Duration,
    /// 加载进程硬超时
    pub reload_hard_timeout: Duration,
    /// 单条日志最大长度 (字节)
    pub log_line_byte: usize,
    /// 单次日志分页查询允许的最大条数
    pub log_page_n: usize,
    /// 日志自动清理间隔
    pub log_clean_interval: Duration,
    /// 日志保留时长
    pub log_retention: Duration,
    /// 丢弃日志警告的最小间隔
    pub warn_interval_min: Duration,
    /// 待写入日志数据总大小上限 (字节)
    pub log_pending_byte: usize,
    /// 日志批发线程内部缓冲区最大条数
    pub log_pending_n: usize,
    /// 日志输入 MPSC 通道容量
    pub log_channel_capacity_n: usize,
    /// 数据库读写线程的请求缓冲区大小 (条)
    pub db_channel_capacity_n: usize,
    /// 通用 MPSC 通道容量
    pub channel_capacity_n: usize,
    /// 退出时等待启动任务返回结果的最长等待时间
    pub startup_wait_timeout: Duration,
}

static SYSD_LIMIT_DEFAULT: SysdLimit = SysdLimit {
    conf_total_byte: SYSD_CONF_LIMIT_TOTAL_BYTE,
    unit_name_byte: SYSD_LIMIT_UNIT_NAME_BYTE,
    unit_key_byte: SYSD_LIMIT_UNIT_KEY_BYTE,
    unit_value_byte: SYSD_LIMIT_UNIT_VALUE_BYTE,
    unit_total_byte: SYSD_LIMIT_UNIT_TOTAL_BYTE,
    ipc_m_byte: SYSD_LIMIT_IPC_M_BYTE,
    ipc_send_timeout: SYSD_LIMIT_IPC_SEND_TIMEOUT_MS,
    ipc_recv_timeout: SYSD_LIMIT_IPC_RECV_TIMEOUT_MS,
    ipc_conn_n: SYSD_LIMIT_IPC_CONN_N,
    ipc_conn_ro_n: SYSD_LIMIT_IPC_CONN_RO_N,
    ipc_handshake_timeout: SYSD_LIMIT_IPC_HANDSHAKE_TIMEOUT_MS,
    ipc_idle_timeout: SYSD_LIMIT_IPC_IDLE_TIMEOUT_MS,
    reload_soft_timeout: SYSD_LIMIT_RELOAD_SOFT_TIMEOUT_MS,
    reload_hard_timeout: SYSD_LIMIT_RELOAD_HARD_TIMEOUT_MS,
    log_line_byte: SYSD_LIMIT_LOG_LINE_BYTE,
    log_page_n: SYSD_LIMIT_LOG_PAGE_N,
    log_clean_interval: SYSD_LIMIT_LOG_CLEAN_INTERVAL_M,
    log_retention: SYSD_LIMIT_LOG_RETENTION_M,
    warn_interval_min: SYSD_LIMIT_WARN_INTERVAL_MIN_MS,
    log_pending_byte: SYSD_LIMIT_LOG_PENDING_BYTE,
    log_pending_n: SYSD_LIMIT_LOG_PENDING_N,
    log_channel_capacity_n: SYSD_LIMIT_LOG_CHANNEL_CAPACITY_N,
    db_channel_capacity_n: SYSD_LIMIT_DB_CHANNEL_CAPACITY_N,
    channel_capacity_n: SYSD_LIMIT_CHANNEL_CAPACITY_N,
    startup_wait_timeout: SYSD_LIMIT_STARTUP_WAIT_TIMEOUT_MS,
};

impl Default for SysdLimit {
    fn default() -> Self {
        SYSD_LIMIT_DEFAULT.clone()
    }
}

impl SysdLimit {
    /// 从常量同名环境变量读取值.
    ///
    /// 环境变量存在但解析失败时, 输出警告并忽略, 使用默认值.
    /// 此方法只在程序启动 / 初始化时调用一次, 之后这些值不再变化.
    pub fn from_env() -> Self {
        let mut l = Self::default();
        env_usize(&mut l.conf_total_byte, "SYSD_CONF_LIMIT_TOTAL_BYTE");
        env_usize(&mut l.unit_name_byte, "SYSD_LIMIT_UNIT_NAME_BYTE");
        env_usize(&mut l.unit_key_byte, "SYSD_LIMIT_UNIT_KEY_BYTE");
        env_usize(&mut l.unit_value_byte, "SYSD_LIMIT_UNIT_VALUE_BYTE");
        env_usize(&mut l.unit_total_byte, "SYSD_LIMIT_UNIT_TOTAL_BYTE");
        env_usize(&mut l.ipc_m_byte, "SYSD_LIMIT_IPC_M_BYTE");
        env_millis(&mut l.ipc_send_timeout, "SYSD_LIMIT_IPC_SEND_TIMEOUT_MS");
        env_millis(&mut l.ipc_recv_timeout, "SYSD_LIMIT_IPC_RECV_TIMEOUT_MS");
        env_usize(&mut l.ipc_conn_n, "SYSD_LIMIT_IPC_CONN_N");
        env_usize(&mut l.ipc_conn_ro_n, "SYSD_LIMIT_IPC_CONN_RO_N");
        env_millis(
            &mut l.ipc_handshake_timeout,
            "SYSD_LIMIT_IPC_HANDSHAKE_TIMEOUT_MS",
        );
        env_millis(&mut l.ipc_idle_timeout, "SYSD_LIMIT_IPC_IDLE_TIMEOUT_MS");
        env_millis(
            &mut l.reload_soft_timeout,
            "SYSD_LIMIT_RELOAD_SOFT_TIMEOUT_MS",
        );
        env_millis(
            &mut l.reload_hard_timeout,
            "SYSD_LIMIT_RELOAD_HARD_TIMEOUT_MS",
        );
        env_usize(&mut l.log_line_byte, "SYSD_LIMIT_LOG_LINE_BYTE");
        env_usize(&mut l.log_page_n, "SYSD_LIMIT_LOG_PAGE_N");
        env_minutes(&mut l.log_clean_interval, "SYSD_LIMIT_LOG_CLEAN_INTERVAL_M");
        env_minutes(&mut l.log_retention, "SYSD_LIMIT_LOG_RETENTION_M");
        env_millis(&mut l.warn_interval_min, "SYSD_LIMIT_WARN_INTERVAL_MIN_MS");
        env_usize(&mut l.log_pending_byte, "SYSD_LIMIT_LOG_PENDING_BYTE");
        env_usize(&mut l.log_pending_n, "SYSD_LIMIT_LOG_PENDING_N");
        env_usize(
            &mut l.log_channel_capacity_n,
            "SYSD_LIMIT_LOG_CHANNEL_CAPACITY_N",
        );
        env_usize(
            &mut l.db_channel_capacity_n,
            "SYSD_LIMIT_DB_CHANNEL_CAPACITY_N",
        );
        env_usize(&mut l.channel_capacity_n, "SYSD_LIMIT_CHANNEL_CAPACITY_N");
        env_millis(
            &mut l.startup_wait_timeout,
            "SYSD_LIMIT_STARTUP_WAIT_TIMEOUT_MS",
        );
        l
    }

    /// 本结构中的 IPC 收发限制
    pub fn ipc(&self) -> IpcLimit {
        IpcLimit {
            m_byte: self.ipc_m_byte,
            send_timeout: self.ipc_send_timeout,
            recv_timeout: self.ipc_recv_timeout,
        }
    }
}

/// 所有 pm-sysd-ctl / pm-sysd-log 使用的限制值.
///
/// 字段名省略 `SYSD_LIMIT_` 前缀, 默认值取自本模块同名常量.
#[derive(Debug, Clone)]
pub struct SysdClientLimit {
    /// 单条 IPC 消息最大长度 (字节)
    pub ipc_m_byte: usize,
    /// 单条 IPC 消息发送超时
    pub ipc_send_timeout: Duration,
    /// 单条 IPC 消息接收超时
    pub ipc_recv_timeout: Duration,
    /// Windows 命名管道连接快速失败的总耗时上限
    pub ipc_connect_timeout: Duration,
    /// Windows 命名管道连接遇到 ERROR_PIPE_BUSY 时的重试间隔
    pub ipc_connect_retry_interval: Duration,
}

static SYSD_CLIENT_LIMIT_DEFAULT: SysdClientLimit = SysdClientLimit {
    ipc_m_byte: SYSD_LIMIT_IPC_M_BYTE,
    ipc_send_timeout: SYSD_LIMIT_IPC_SEND_TIMEOUT_MS,
    ipc_recv_timeout: SYSD_LIMIT_IPC_RECV_TIMEOUT_MS,
    ipc_connect_timeout: SYSD_LIMIT_IPC_CONNECT_TIMEOUT_MS,
    ipc_connect_retry_interval: SYSD_LIMIT_IPC_CONNECT_RETRY_INTERVAL_MS,
};

impl Default for SysdClientLimit {
    fn default() -> Self {
        SYSD_CLIENT_LIMIT_DEFAULT.clone()
    }
}

impl SysdClientLimit {
    /// 从常量同名环境变量读取值.
    ///
    /// 环境变量存在但解析失败时, 输出警告并忽略, 使用默认值.
    /// 此方法只在程序启动 / 初始化时调用一次, 之后这些值不再变化.
    pub fn from_env() -> Self {
        let mut l = Self::default();
        env_usize(&mut l.ipc_m_byte, "SYSD_LIMIT_IPC_M_BYTE");
        env_millis(&mut l.ipc_send_timeout, "SYSD_LIMIT_IPC_SEND_TIMEOUT_MS");
        env_millis(&mut l.ipc_recv_timeout, "SYSD_LIMIT_IPC_RECV_TIMEOUT_MS");
        env_millis(
            &mut l.ipc_connect_timeout,
            "SYSD_LIMIT_IPC_CONNECT_TIMEOUT_MS",
        );
        env_millis(
            &mut l.ipc_connect_retry_interval,
            "SYSD_LIMIT_IPC_CONNECT_RETRY_INTERVAL_MS",
        );
        l
    }

    /// 本结构中的 IPC 收发限制
    pub fn ipc(&self) -> IpcLimit {
        IpcLimit {
            m_byte: self.ipc_m_byte,
            send_timeout: self.ipc_send_timeout,
            recv_timeout: self.ipc_recv_timeout,
        }
    }
}

/// 读取并解析环境变量, 解析失败返回 `None` 并输出警告.
fn read_env<T>(name: &str) -> Option<T>
where
    T: FromStr,
{
    let v = env::var(name).ok()?;
    match v.parse() {
        Ok(x) => Some(x),
        Err(_) => {
            log::warn!(
                "invalid environment variable `{} = {:?}`. using default",
                name,
                v
            );
            None
        }
    }
}

/// 读取 `usize` 类型环境变量
fn env_usize(dst: &mut usize, name: &str) {
    if let Some(v) = read_env::<usize>(name) {
        *dst = v;
    }
}

/// 读取毫秒类型环境变量 (值为毫秒数)
fn env_millis(dst: &mut Duration, name: &str) {
    if let Some(v) = read_env::<u64>(name) {
        *dst = Duration::from_millis(v);
    }
}

/// 读取分钟类型环境变量 (值为分钟数)
fn env_minutes(dst: &mut Duration, name: &str) {
    if let Some(v) = read_env::<u64>(name) {
        *dst = Duration::from_secs(v.saturating_mul(60));
    }
}
