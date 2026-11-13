//! 服务管理 核心逻辑.
//!
//! 管理由 pm-sysd-init 启动的 进程 (service). 初版只支持 service 一种 unit 类型.
//!
//! 采用多 actor 架构: 每个关键组件是 `tokio::spawn` 的 async task, 持有私有状态.
//! 组件之间通过 channel 消息通信 (事件驱动).
//!
//!
//! ## 状态枚举
//!
//! 尽量与 systemd 保持一致, 不要随便修改.
//!
//! - `UnitSta`: systemd 定义的所有 unit 的可能状态.
//! - `ServiceSta`: systemd 定义的所有 service 的可能状态.
//! - `TimerSta`: systemd 定义的所有 timer 的可能状态.
//! - `UnitSubSta`: 用于包装 `ServiceSta` 与 `TimerSta`.
//!
//! `ServiceSta` 和 `TimerSta` 可以 (有损) 映射到 `UnitSta`.

use std::fmt;

use serde::{Deserialize, Serialize};

pub mod init;

mod error;
mod service_d;
mod service_s;
mod sys_d;
mod timer_d;
mod timer_s;
mod unit_d;

#[cfg(test)]
mod test;

pub use error::SysdError;
pub use sys_d::{SysD, SysDM};

/// 内置 target: `default.target`
///
/// 平时正常运行时的 target.
pub const TARGET_DEFAULT: &str = "default.target";
/// 内置 target: `exit.target`
///
/// 退出时的 target.
pub const TARGET_EXIT: &str = "exit.target";

/// unit 的激活状态 (对应 systemd 的 `UnitActiveState`).
///
/// 这是 systemd 定义的所有 unit 的可能状态.
///
/// 当前版本 实际只使用 `Active`, `Inactive`, `Failed`.
/// 其余状态为后续机制预留.
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
    /// reloading (unit 正在 reload)
    Reloading,
    /// 正在刷新资源
    Refreshing,
}

/// service 的子状态 (对应 systemd 的 `ServiceState`).
///
/// 这是 systemd 定义的所有 service 的可能状态.
/// 当前版本只使用其中一部分.
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
    /// 自动重启前 的 dead
    DeadBeforeAutoRestart,
    /// 自动重启前 的 failed
    FailedBeforeAutoRestart,
    /// 资源被保留 的 dead
    DeadResourcesPinned,
    /// 自动重启中
    AutoRestart,
    /// 自动重启 已排队
    AutoRestartQueued,
    /// 清理中
    Cleaning,
}

/// timer 的子状态 (对应 systemd 的 `TimerState`).
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

/// target 状态 (保留)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetSta {
    /// 已激活 (target 已到达)
    Active,
}

/// unit 子状态 (按 unit 类型区分).
///
/// 用于包装 [`ServiceSta`] 与 [`TimerSta`].
///
/// 序列化为子状态字符串 (例如 `"running"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UnitSubSta {
    /// service unit 子状态
    Service(ServiceSta),
    /// timer unit 子状态
    Timer(TimerSta),
    /// target unit
    Target(TargetSta),
}

impl UnitSta {
    /// 显示/序列化 文本.
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
            Self::Active => "active",
        }
    }
}

impl UnitSubSta {
    /// 显示/序列化文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Service(s) => s.as_str(),
            Self::Timer(s) => s.as_str(),
            Self::Target(s) => s.as_str(),
        }
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

impl fmt::Display for UnitSubSta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
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

/// `TargetSta` -> `UnitSta` (保留)
impl From<TargetSta> for UnitSta {
    fn from(s: TargetSta) -> Self {
        match s {
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
