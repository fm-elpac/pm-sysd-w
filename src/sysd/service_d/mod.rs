//! ServiceD: 管理所有 service (service 集合).
//!
//! ServiceD 是 `tokio::spawn` 的 async task, 内部持有纯计算状态机 (`service_d::sm`)
//! 与活跃 ServiceS 的句柄映射; actor (`service_d::actor`) 只执行状态机输出的 IO
//! 请求. 查询类请求只读取状态, 由 actor 直接组合句柄与状态机数据回复.
//!
//! ServiceS 由 ServiceD actor 直接创建 / 启动: actor 先同步登记句柄与 active 状态,
//! 再投递初始命令, 避免创建与 ServiceS 退出之间的异步竞态.

use serde::Serialize;

use crate::{conf::Unit, jsonl::SdReqUnitOp};

mod actor;
mod query;
mod sm;

pub use actor::{MS, ServiceD, ServiceDM};
pub use sm::ServiceOp;

/// 单个 unit 操作 (统一结构)
#[derive(Debug, Clone, Serialize)]
pub struct ServiceCmdUnit {
    /// unit 名称 (ResetFailed 时 "" 表示全部)
    pub name: String,
    /// 当前配置中的 unit 配置 (None = daemon-reload 后已从配置中移除)
    pub conf: Option<Unit>,
    /// 具体操作 (复用 SdReqUnitOp)
    pub op: SdReqUnitOp,
}
