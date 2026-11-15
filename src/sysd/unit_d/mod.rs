//! UnitD: 管理所有 unit (内存配置) 与 service (通过 ServiceD).
//!
//! UnitD 是 `tokio::spawn` 的 async task, 内部持有纯计算状态机 (`unit_d::sm`),
//! actor (`unit_d::actor`) 只执行状态机输出的 IO 请求.
//!
//! UnitS (unit 配置与 enable/mask 状态) 由状态机持有; sqlite 数据库仅用于
//! enable/mask 持久化, 启动时读取, 平时通过 WriteDb 动作更新.

mod actor;
mod load;
mod query;
mod sm;
mod unit_s;

#[cfg(test)]
mod test;

pub use actor::{UnitD, UnitDM};
