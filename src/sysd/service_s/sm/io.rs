//! ServiceS 状态机输入输出类型.

use serde::Serialize;

use crate::{
    sysd::{SysdError, service_d::ServiceOp},
    sysu::{ServiceSta, UnitSta},
};

/// 状态机输入数据 (事件)
#[derive(Debug, Clone, Serialize)]
pub enum SmInput {
    /// 外部命令 (复用 ServiceOp)
    Cmd(ServiceOp),
    /// 查询 active / sub 状态
    QueryState,
    /// 查询详细状态
    QueryStatus,
    /// 子进程已创建 (携带 pid)
    Spawned { pid: Option<u32> },
    /// 子进程创建失败 (携带错误)
    SpawnFailed(SysdError),
    /// 子进程已退出 (自然退出或强制终止)
    ChildExited(ExitInfo),
    /// ExecStop 执行完成 (失败已由 actor 记录日志)
    ExecStopDone,
    /// ExecReload 执行完成 (携带结果)
    ReloadDone(Result<(), SysdError>),
    /// 重启定时器到期
    RestartTimer,
    /// 启动超时 (仅 Type=oneshot 使用)
    StartTimeout,
}

/// 状态机输出的动作 (由 ServiceS actor 执行)
#[derive(Debug, Clone, Serialize)]
pub enum SmOutput {
    /// 立即返回 Cmd 响应
    ReplyCmd(Result<(), SysdError>),
    /// 返回 QueryState 响应
    ReplyState((UnitSta, ServiceSta)),
    /// 返回 QueryStatus 响应
    ReplyStatus(ServiceStatusInfo),
    /// 启动子进程
    SpawnChild,
    /// 终止子进程并等待其退出
    KillChild,
    /// 执行 ExecStop
    RunExecStop,
    /// 执行 ExecReload
    RunExecReload,
    /// 安排定时重启
    ScheduleRestart { secs: u32 },
    /// 取消定时重启
    CancelRestart,
    /// 安排启动超时
    ScheduleStartTimeout { secs: u32 },
    /// 取消启动超时
    CancelStartTimeout,
    /// 请求退出并上报终止状态 (ServiceS 不自行退出, 由 ServiceD 回发 Exit)
    RequestExit { status: ServiceStatus },
}

/// 一个 service 的详细运行状态快照 (供 `QueryStatus` 回复).
///
/// 同时用于 ServiceS 状态机查询结果与 ServiceD 的 status 查询, 避免在各层之间
/// 传递冗长的元组.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ServiceStatusInfo {
    /// 激活状态
    pub active: UnitSta,
    /// 子状态
    pub sub: ServiceSta,
    /// 运行中进程 pid
    pub pid: Option<u32>,
    /// 自动重启次数
    pub restart_count: u32,
    /// 最后一次退出状态码
    pub exit_status: Option<i32>,
    /// 当前状态进入时间 (Unix 毫秒时间戳)
    pub since: Option<i64>,
}

impl ServiceStatusInfo {
    /// 未运行 (inactive / dead) 的默认快照
    pub fn inactive() -> Self {
        Self {
            active: UnitSta::Inactive,
            sub: ServiceSta::Dead,
            pid: None,
            restart_count: 0,
            exit_status: None,
            since: None,
        }
    }
}

/// 子进程退出信息 (纯数据)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExitInfo {
    /// 是否成功退出 (退出码 0)
    pub success: bool,
    /// 退出状态码
    pub code: Option<i32>,
}

/// 一个 service 的终止状态 (上报给 ServiceD, 用于保留最近状态)
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ServiceStatus {
    /// 是否 failed
    pub failed: bool,
    /// 自动重启次数
    pub restart_count: u32,
    /// 最后一次退出状态码
    pub exit_status: Option<i32>,
    /// 当前状态进入时间 (Unix 毫秒时间戳)
    pub since: Option<i64>,
}

impl ServiceStatus {
    /// 由终止状态推导 active / sub 状态
    pub fn active_sub(&self) -> (UnitSta, ServiceSta) {
        if self.failed {
            (UnitSta::Failed, ServiceSta::Failed)
        } else {
            (UnitSta::Inactive, ServiceSta::Dead)
        }
    }
}
