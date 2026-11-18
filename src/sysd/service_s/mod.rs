//! ServiceS: 管理单个 service (启动, 停止, 重启, reload 等).
//!
//! ServiceS 是 `tokio::spawn` 的 async task, 只管理一个 service 的动态运行状态.
//! 内部持有纯计算状态机 (`service_s::sm`), actor (`service_s::actor`) 只执行状态机
//! 输出的 IO 请求. 进程相关逻辑位于 `proc`.

mod actor;
mod proc;
mod sm;

pub use actor::{ServiceS, ServiceSM};
pub use sm::{ServiceStatus, ServiceStatusInfo};
