//! ServiceD: 管理所有 service (service 集合).
//!
//! ServiceD 是 `tokio::spawn` 的 async task, 内部持有纯计算状态机 (`service_d::sm`)
//! 与活跃 ServiceS 的句柄映射; actor (`service_d::actor`) 只执行状态机输出的 IO
//! 请求. 查询类请求只读取状态, 由 actor 直接组合句柄与状态机数据回复.
//!
//! ServiceS 的创建 (新启动) 不直接在主循环中执行, 而是交给一个专用 job 队列
//! (另一个 async task) 串行处理: 主循环只入队创建 job (携带初始消息), job 队列
//! 创建 ServiceS 后无阻塞转发初始消息 (不等待执行完成).

use crate::{
    conf::Unit,
    jsonl::SdReqUnitOp,
    sysd::{ServiceSta, UnitSta},
};

mod actor;
mod query;
mod sm;

#[cfg(test)]
mod test;

pub use actor::{ServiceD, ServiceDM};
pub use sm::ServiceOp;

/// service 详细状态快照
#[derive(Debug, Clone, Copy)]
pub struct StatusInfo {
    pub active: UnitSta,
    pub sub: ServiceSta,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub exit_status: Option<i32>,
}

/// 单个 unit 操作 (统一结构)
#[derive(Debug, Clone)]
pub struct ServiceCmdUnit {
    /// unit 名称 (ResetFailed 时 "" 表示全部)
    pub name: String,
    /// 当前配置中的 unit 配置 (None = daemon-reload 后已从配置中移除)
    pub conf: Option<Unit>,
    /// 具体操作 (复用 SdReqUnitOp)
    pub op: SdReqUnitOp,
}
