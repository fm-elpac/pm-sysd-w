//! ServiceD 状态机输入输出类型.

use serde::Serialize;

use crate::{
    conf::Unit,
    jsonl::SdRes,
    sysd::{SysdError, service_d::ServiceCmdUnit, service_s::ServiceStatus},
};

/// 状态机输入数据 (事件)
#[derive(Debug, Clone, Serialize)]
pub enum SmInput {
    /// 单个 unit 操作
    Unit(ServiceCmdUnit),
    /// 批量启动 (default.target)
    StartMany(Vec<(String, Unit)>),
    /// 停止所有 service 并退出
    Exit,
    /// ServiceD actor 已直接创建 ServiceS (携带 name 与配置)
    ServiceStarted { name: String, conf: Unit },
    /// ServiceS 请求退出 (携带状态)
    ServiceTerminated { name: String, status: ServiceStatus },
    /// 后台 task 已完成停止所有 service
    StopAllDone(Result<(), SysdError>),
}

/// 状态机输出的动作 (由 ServiceD actor 执行)
#[derive(Debug, Clone, Serialize)]
pub enum SmOutput {
    /// 立即返回 SdReq 响应 (Unit 命令路径)
    Reply(SdRes),
    /// 立即返回 StartMany / Exit 响应
    ReplyResult(Result<(), SysdError>),
    /// 转发命令给单个 ServiceS.
    ///
    /// Unit 命令路径等待回执并回复调用方; StartMany 路径不等待回执, 失败仅记录日志.
    SendServiceS { name: String, op: ServiceOp },
    /// 转发命令给全部 ServiceS (Unit 命令路径, 汇总回复)
    SendServiceSAll { op: ServiceOp },
    /// 直接创建并启动 ServiceS.
    ///
    /// Unit 命令路径等待回执并回复调用方.
    /// StartMany 路径不等待回执, 失败仅记录日志.
    StartService { name: String, conf: Unit },
    /// 停止所有 service (等待完成)
    StopAll,
    /// 退出 ServiceD 主循环
    Exit,
}

/// 转发给 ServiceS 的单个命令 (不含回执, 由 ServiceD 执行时注入回执)
#[derive(Debug, Clone, Copy, Serialize)]
pub enum ServiceOp {
    Start,
    Stop,
    Kill,
    Restart,
    TryRestart,
    Reload,
    ReloadOrRestart,
    TryReloadOrRestart,
    ResetFailed,
}
