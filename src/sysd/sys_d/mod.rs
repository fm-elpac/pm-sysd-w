//! SysD: 整个 pm-sysd-init 管理实例 (顶级实体).
//!
//! SysD 是 `tokio::spawn` 的 async task, 内部持有纯计算状态机 (`sys_d::sm`),
//! actor (`sys_d::actor`) 只执行状态机输出的 IO 请求.
//!
//! - 自己处理 `SdReq::SysD` (exit / soft-reboot / sqlite-vacuum).
//! - 日志查询 `SdReq::LogRO` 转发给 Logd; 其余 ctl 请求与 `StartDefaultTarget`
//!   原样转发给 UnitD.
//!
//! 转发不阻塞 SysD 主循环: 每条消息自带 oneshot 回执, 转发给 Logd / UnitD 时
//! 直接透传 (仅等待消息投递完成, 不等待处理/回执). 只有处理 `SdReq::SysD` 中的
//! exit / soft-reboot 时, SysD 才会等待 UnitD 停止所有 unit. `sqlite-vacuum`
//! 同样不阻塞主循环: 由独立 task 投递给数据库读写线程处理.
//!
//! 退出时: 先通知 UnitD 停止所有 unit, 再通过 `exit_tx` (oneshot) 通知
//! `sysd::init`, 最后退出自己的循环.

mod actor;
mod sm;

pub use actor::{SysD, SysDM};
