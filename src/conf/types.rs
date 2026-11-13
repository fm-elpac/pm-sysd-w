//! 配置 数据类型 (unit / service / install / win / 加载结果).

use serde::{Deserialize, Serialize};

/// unit 类型 (模仿 systemd).
///
/// pm-sysd-w 只计划支持 `service` / `timer` / `target` 三种; 当前版本 只实现
/// `service`, 不实现 `timer`, 也不支持 自定义 `target`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnitType {
    /// service unit.
    Service,
    /// timer unit (当前版本 未实现).
    Timer,
    /// target unit (当前版本 不支持 自定义 target).
    Target,
}

impl UnitType {
    /// 配置关键字 (用于 显示/序列化).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Service => "service",
            Self::Timer => "timer",
            Self::Target => "target",
        }
    }
}

/// 一个 unit (service) 的配置 (模仿 systemd unit 文件 的 节).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unit {
    /// unit 名称 (例如 `test-run.service`)
    pub name: String,
    /// unit 类型 (当前版本 仅 `service`)
    pub unit_type: UnitType,
    /// 描述
    pub description: String,
    /// 启动限流 时间窗口 (秒) (默认 10; 对应功能 暂未实现)
    pub start_limit_interval_sec: u32,
    /// 启动限流 次数 (默认 5; 对应功能 暂未实现)
    pub start_limit_burst: u32,
    /// `[Service]` 节
    pub service: Service,
    /// `[Install]` 节
    pub install: Install,
    /// `[Win]` 节 (pm-sysd-w 专属扩展)
    pub win: Win,
}

/// service 启动类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceType {
    /// 默认: 启动即视为 active.
    Simple,
    /// 进程退出 (成功) 视为启动完成, 不持续运行.
    Oneshot,
}

impl ServiceType {
    /// 配置关键字 (用于 显示/序列化).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Oneshot => "oneshot",
        }
    }
}

/// 重启策略.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    /// 不自动重启 (默认).
    No,
    /// 退出后总是重启.
    Always,
    /// 仅异常退出时重启.
    OnFailure,
}

impl RestartPolicy {
    /// 配置关键字 (用于 显示/序列化).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Always => "always",
            Self::OnFailure => "on-failure",
        }
    }
}

/// `[Service]` 节 配置.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    /// 启动类型
    pub service_type: ServiceType,
    /// 启动命令
    pub exec_start: Vec<String>,
    /// 停止命令
    pub exec_stop: Option<Vec<String>>,
    /// reload 命令
    pub exec_reload: Option<Vec<String>>,
    /// 重启策略
    pub restart: RestartPolicy,
    /// 重启间隔 (秒)
    pub restart_sec: u32,
    /// 启动超时 (秒)
    pub timeout_start_sec: u32,
    /// 停止超时 (秒)
    pub timeout_stop_sec: u32,
    /// 工作目录
    pub working_directory: Option<String>,
    /// 环境变量 (`KEY=VALUE` 列表)
    pub environment: Vec<(String, String)>,
    /// 进程退出后是否仍视为 active
    pub remain_after_exit: bool,
}

/// `[Install]` 节 配置.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Install {
    /// 所属 target (显式设置后该 unit 才可 enable)
    pub wanted_by: Option<String>,
}

/// `[Win]` 节 配置 (pm-sysd-w 专属扩展).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Win {
    /// 启动进程时 设置 CREATE_NO_WINDOW 标志 (console 程序不显示 黑窗口).
    ///
    /// 仅 Windows 平台有效, Linux 上 忽略.
    pub create_no_window: bool,
}

/// 加载配置文件 得到的结果.
#[derive(Debug, Clone)]
pub struct Conf {
    /// 所有 unit (service) 配置
    pub units: Vec<Unit>,
    /// sqlite 数据库路径 (绝对路径)
    pub db_path: String,
    /// 主配置文件所在目录 (绝对路径)
    pub conf_dir: String,
    /// 加载时 产生的 警告 (不支持的 `Unit.` / `Service.` / `Install.` / `Timer.` /
    /// `Win.` 前缀 key). 由调用方 通过 `log::warn!()` 输出并写入日志.
    pub warnings: Vec<String>,
}
