//! systemd 风格的状态定义.
//!
//! 本模块存放从 systemd "抄" 过来的状态枚举与映射规则.
//!
//! 尽量与 systemd 保持一致, 不要随便修改.
//!
//! `ServiceSta`, `TimerSta`, `TargetSta` 可以 (有损) 映射到 `UnitSta`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// 整个 pm-sysd-init 管理实例的运行状态 (对应 systemd 的 `ManagerState`,
/// 即 `systemctl is-system-running` 的输出).
///
/// 这是 systemd 定义的所有可能输出. 本项目只使用其中的一部分, 未使用的状态
/// 见各变体注释.
///
/// 尽量与 systemd 保持一致, 不要随便修改.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonSta {
    /// 早期启动阶段 (尚未到达 basic.target, 或进入维护状态).
    ///
    /// 本项目没有 basic.target / 维护状态, 且监听 IPC 之前不接受连接, 暂不使用.
    Initializing,
    /// 后期启动阶段 (job 队列第一次空闲之前, 或到达 rescue target).
    ///
    /// 本项目没有 job 队列, 启动阶段对外不可观察, 暂不使用.
    Starting,
    /// 系统完全运行.
    Running,
    /// 系统正在运行, 但有一个或多个 unit 失败.
    Degraded,
    /// rescue 或 emergency target 处于激活状态.
    ///
    /// 本项目没有 rescue / emergency target, 暂不使用.
    Maintenance,
    /// 管理器正在关闭.
    Stopping,
    /// 管理器未运行 (具体为不兼容的程序作为系统管理器时).
    ///
    /// 本项目用于 IPC 端点不存在 (pm-sysd-init 未运行) 时.
    Offline,
    /// 无法确定运行状态 (资源不足或其它错误).
    ///
    /// 本项目用于查询 ServiceD 状态失败时.
    Unknown,
}

/// unit 的加载状态 (对应 systemd 的 `UnitLoadState`).
///
/// 这是 systemd 定义的所有 unit 的可能状态.
/// 当前版本只使用其中的一部分.
///
/// `systemctl list-units` 的 LOAD 列常见值:
/// `loaded`, `not-found`, `bad-setting`, `error`, `masked`.
///
/// 尽量与 systemd 保持一致, 不要随便修改.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoadSta {
    /// 尚未加载 (内部占位)
    Stub,
    /// 已加载
    Loaded,
    /// unit 文件不存在
    NotFound,
    /// 关键设置解析失败
    BadSetting,
    /// 其它错误
    Error,
    /// 已合并到其它 unit
    Merged,
    /// 已屏蔽
    Masked,
}

/// unit 的启用状态 (对应 systemd 的 `UnitFileState`, 即 `is-enabled` 输出).
///
/// 这是 systemd 定义的所有可能状态.
/// 当前版本只使用其中的一部分.
///
/// 尽量与 systemd 保持一致, 不要随便修改.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnableSta {
    /// 已启用 (持久)
    Enabled,
    /// 已启用 (仅运行时)
    EnabledRuntime,
    /// 已链接 (符号链接到查找路径之外)
    Linked,
    /// 已链接 (仅运行时)
    LinkedRuntime,
    /// 别名
    Alias,
    /// 已屏蔽
    Masked,
    /// 已屏蔽 (仅运行时)
    MaskedRuntime,
    /// 静态 (无 `[Install]` 配置, 不能 enable/disable)
    Static,
    /// 已禁用
    Disabled,
    /// 间接启用 (被其它 unit 依赖)
    Indirect,
    /// 生成 (generator 产生)
    Generated,
    /// 临时 (transient unit)
    Transient,
    /// 无效 (unit 文件损坏)
    Bad,
}

/// unit 的激活状态 (对应 systemd 的 `UnitActiveState`).
///
/// 这是 systemd 定义的所有 unit 的可能状态.
/// 当前版本只使用其中的一部分.
///
/// 尽量与 systemd 保持一致, 不要随便修改.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UnitSta {
    /// 已激活
    Active,
    /// 未激活
    Inactive,
    /// 失败
    Failed,
    /// 正在激活
    Activating,
    /// 正在停用
    Deactivating,
    /// 维护中
    Maintenance,
    /// 重新加载中 (unit 正在 reload)
    Reloading,
    /// 正在刷新资源
    Refreshing,
}

/// service 的状态 (对应 systemd 的 `ServiceState`).
///
/// 这是 systemd 定义的所有 service 的可能状态.
/// 当前版本只使用其中的一部分.
///
/// 尽量与 systemd 保持一致, 不要随便修改.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ServiceSta {
    /// 未运行
    Dead,
    /// 检查启动条件
    Condition,
    /// ExecStartPre 阶段
    StartPre,
    /// 启动阶段
    Start,
    /// ExecStartPost 阶段
    StartPost,
    /// 主进程运行中
    Running,
    /// 运行中, 等待 D-Bus 名称重新出现
    RunningRevalidating,
    /// 已退出 (RemainAfterExit=yes)
    Exited,
    /// reload 时刷新 extensions
    RefreshExtensions,
    /// reload 时刷新 credentials
    RefreshCredentials,
    /// 通过 ExecReload= 执行 reload
    Reload,
    /// 通过 SIGHUP 执行 reload
    ReloadSignal,
    /// 等待 RELOADING=1 后的 READY=1
    ReloadNotify,
    /// ExecReloadPost 阶段
    ReloadPost,
    /// 挂载到 service namespace 中
    Mounting,
    /// 停止阶段
    Stop,
    /// 停止 watchdog 阶段
    StopWatchdog,
    /// 停止 SIGTERM 阶段
    StopSigterm,
    /// 停止 SIGKILL 阶段
    StopSigkill,
    /// ExecStopPost 阶段
    StopPost,
    /// 最终 watchdog 阶段
    FinalWatchdog,
    /// 最终 SIGTERM 阶段
    FinalSigterm,
    /// 最终 SIGKILL 阶段
    FinalSigkill,
    /// 失败
    Failed,
    /// 自动重启前的 dead
    DeadBeforeAutoRestart,
    /// 自动重启前的 failed
    FailedBeforeAutoRestart,
    /// 资源被保留的 dead
    DeadResourcesPinned,
    /// 自动重启中
    AutoRestart,
    /// 自动重启已排队
    AutoRestartQueued,
    /// 清理中
    Cleaning,
}

/// timer 的状态 (对应 systemd 的 `TimerState`).
///
/// 这是 systemd 定义的所有 timer 的可能状态.
/// 当前版本未实现 timer, 枚举为后续机制预留.
///
/// 尽量与 systemd 保持一致, 不要随便修改.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TimerSta {
    /// 未激活
    Dead,
    /// 等待下一次触发
    Waiting,
    /// 正在激活关联 unit
    Running,
    /// 已触发过且不再触发
    Elapsed,
    /// 失败
    Failed,
}

/// target 的状态 (对应 systemd 的 `TargetState`).
///
/// 这是 systemd 定义的所有 target 的可能状态.
///
/// 尽量与 systemd 保持一致, 不要随便修改.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetSta {
    /// 未激活 (target 未到达)
    Dead,
    /// 已激活 (target 已到达)
    Active,
}

/// unit 子状态 (按 unit 类型区分).
///
/// 用于包装 [`ServiceSta`] 与 [`TimerSta`] 等.
///
/// 序列化时带类型标记 (例如 `{"type": "service", "data": "running"}`),
/// 反序列化可无损还原类型, 不会把 timer 误还原为 service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "kebab-case")]
pub enum UnitSubSta {
    /// service unit
    Service(ServiceSta),
    /// timer unit
    Timer(TimerSta),
    /// target unit
    Target(TargetSta),
}

impl DaemonSta {
    /// 显示/序列化文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Initializing => "initializing",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Degraded => "degraded",
            Self::Maintenance => "maintenance",
            Self::Stopping => "stopping",
            Self::Offline => "offline",
            Self::Unknown => "unknown",
        }
    }

    /// `is-system-running` 退出码 (模仿 systemctl).
    ///
    /// 只有 `running` 返回 0, 其余状态返回 1 (systemctl 实际返回
    /// `EXIT_FAILURE`, 即 1).
    pub fn exit_code(&self) -> u8 {
        if matches!(self, Self::Running) { 0 } else { 1 }
    }
}

impl LoadSta {
    /// 显示/序列化文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stub => "stub",
            Self::Loaded => "loaded",
            Self::NotFound => "not-found",
            Self::BadSetting => "bad-setting",
            Self::Error => "error",
            Self::Merged => "merged",
            Self::Masked => "masked",
        }
    }
}

impl EnableSta {
    /// 显示/序列化文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::EnabledRuntime => "enabled-runtime",
            Self::Linked => "linked",
            Self::LinkedRuntime => "linked-runtime",
            Self::Alias => "alias",
            Self::Masked => "masked",
            Self::MaskedRuntime => "masked-runtime",
            Self::Static => "static",
            Self::Disabled => "disabled",
            Self::Indirect => "indirect",
            Self::Generated => "generated",
            Self::Transient => "transient",
            Self::Bad => "bad",
        }
    }
}

impl UnitSta {
    /// 显示/序列化文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
            Self::Failed => "failed",
            Self::Activating => "activating",
            Self::Deactivating => "deactivating",
            Self::Maintenance => "maintenance",
            Self::Reloading => "reloading",
            Self::Refreshing => "refreshing",
        }
    }
}

impl ServiceSta {
    /// 显示/序列化文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dead => "dead",
            Self::Condition => "condition",
            Self::StartPre => "start-pre",
            Self::Start => "start",
            Self::StartPost => "start-post",
            Self::Running => "running",
            Self::RunningRevalidating => "running-revalidating",
            Self::Exited => "exited",
            Self::RefreshExtensions => "refresh-extensions",
            Self::RefreshCredentials => "refresh-credentials",
            Self::Reload => "reload",
            Self::ReloadSignal => "reload-signal",
            Self::ReloadNotify => "reload-notify",
            Self::ReloadPost => "reload-post",
            Self::Mounting => "mounting",
            Self::Stop => "stop",
            Self::StopWatchdog => "stop-watchdog",
            Self::StopSigterm => "stop-sigterm",
            Self::StopSigkill => "stop-sigkill",
            Self::StopPost => "stop-post",
            Self::FinalWatchdog => "final-watchdog",
            Self::FinalSigterm => "final-sigterm",
            Self::FinalSigkill => "final-sigkill",
            Self::Failed => "failed",
            Self::DeadBeforeAutoRestart => "dead-before-auto-restart",
            Self::FailedBeforeAutoRestart => "failed-before-auto-restart",
            Self::DeadResourcesPinned => "dead-resources-pinned",
            Self::AutoRestart => "auto-restart",
            Self::AutoRestartQueued => "auto-restart-queued",
            Self::Cleaning => "cleaning",
        }
    }
}

impl TimerSta {
    /// 显示/序列化文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dead => "dead",
            Self::Waiting => "waiting",
            Self::Running => "running",
            Self::Elapsed => "elapsed",
            Self::Failed => "failed",
        }
    }
}

impl TargetSta {
    /// 显示/序列化文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dead => "dead",
            Self::Active => "active",
        }
    }
}

impl UnitSubSta {
    /// 内部状态文本 (不含外层状态)
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Service(s) => s.as_str(),
            Self::Timer(s) => s.as_str(),
            Self::Target(s) => s.as_str(),
        }
    }
}

impl fmt::Display for DaemonSta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for LoadSta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for EnableSta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for UnitSta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for ServiceSta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for TimerSta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for TargetSta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 模仿 systemd 的显示格式, 比如 `active(exited)`
impl fmt::Display for UnitSubSta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", UnitSta::from(*self).as_str(), self.as_str())
    }
}

/// `ServiceSta` -> `UnitSta` 的 (有损) 映射.
///
/// 映射规则与 systemd 保持一致.
/// (参考 systemd `src/core/service.c` 的 `state_translation_table`)
impl From<ServiceSta> for UnitSta {
    fn from(s: ServiceSta) -> Self {
        match s {
            ServiceSta::Dead => UnitSta::Inactive,
            ServiceSta::Condition => UnitSta::Activating,
            ServiceSta::StartPre => UnitSta::Activating,
            ServiceSta::Start => UnitSta::Activating,
            ServiceSta::StartPost => UnitSta::Activating,
            ServiceSta::Running => UnitSta::Active,
            ServiceSta::RunningRevalidating => UnitSta::Active,
            ServiceSta::Exited => UnitSta::Active,
            ServiceSta::RefreshExtensions => UnitSta::Refreshing,
            ServiceSta::RefreshCredentials => UnitSta::Refreshing,
            ServiceSta::Reload => UnitSta::Reloading,
            ServiceSta::ReloadSignal => UnitSta::Reloading,
            ServiceSta::ReloadNotify => UnitSta::Reloading,
            ServiceSta::ReloadPost => UnitSta::Reloading,
            ServiceSta::Mounting => UnitSta::Refreshing,
            ServiceSta::Stop => UnitSta::Deactivating,
            ServiceSta::StopWatchdog => UnitSta::Deactivating,
            ServiceSta::StopSigterm => UnitSta::Deactivating,
            ServiceSta::StopSigkill => UnitSta::Deactivating,
            ServiceSta::StopPost => UnitSta::Deactivating,
            ServiceSta::FinalWatchdog => UnitSta::Deactivating,
            ServiceSta::FinalSigterm => UnitSta::Deactivating,
            ServiceSta::FinalSigkill => UnitSta::Deactivating,
            ServiceSta::Failed => UnitSta::Failed,
            ServiceSta::DeadBeforeAutoRestart => UnitSta::Inactive,
            ServiceSta::FailedBeforeAutoRestart => UnitSta::Failed,
            ServiceSta::DeadResourcesPinned => UnitSta::Inactive,
            ServiceSta::AutoRestart => UnitSta::Activating,
            ServiceSta::AutoRestartQueued => UnitSta::Activating,
            ServiceSta::Cleaning => UnitSta::Maintenance,
        }
    }
}

/// `TimerSta` -> `UnitSta` 的 (有损) 映射.
///
/// 映射规则与 systemd 保持一致.
/// (参考 systemd `src/core/timer.c` 的 `state_translation_table`)
impl From<TimerSta> for UnitSta {
    fn from(s: TimerSta) -> Self {
        match s {
            TimerSta::Dead => UnitSta::Inactive,
            TimerSta::Waiting => UnitSta::Active,
            TimerSta::Running => UnitSta::Active,
            TimerSta::Elapsed => UnitSta::Active,
            TimerSta::Failed => UnitSta::Failed,
        }
    }
}

/// `TargetSta` -> `UnitSta` 的 (有损) 映射.
///
/// 映射规则与 systemd 保持一致.
/// (参考 systemd `src/core/target.c` 的 `state_translation_table`)
impl From<TargetSta> for UnitSta {
    fn from(s: TargetSta) -> Self {
        match s {
            TargetSta::Dead => UnitSta::Inactive,
            TargetSta::Active => UnitSta::Active,
        }
    }
}

/// `UnitSubSta` -> `UnitSta` 的 (有损) 映射.
impl From<UnitSubSta> for UnitSta {
    fn from(s: UnitSubSta) -> Self {
        match s {
            UnitSubSta::Service(s) => s.into(),
            UnitSubSta::Timer(s) => s.into(),
            UnitSubSta::Target(s) => s.into(),
        }
    }
}
