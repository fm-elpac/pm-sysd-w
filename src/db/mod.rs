//! 数据库 (sqlite) 相关功能的抽象/封装.
//!
//! pm-sysd-init 使用 sqlite 数据库存储持久状态 (例如 unit 是否 enable/mask),
//! 以及日志数据. 数据库格式属于内部 API, 不作为外部稳定 API 提供.
//!
//! 数据库 schema 见 `schema` 子模块, 版本与升级规则见 `doc/dev/db.md`.
//!
//! ## 线程模型
//!
//! sqlite 访问只发生在 3 个专用 (同步) 线程中:
//!
//! - 数据库读写线程: 处理 [`DbClient`] 通过 MPSC 发来的读写请求,
//!   并把结果通过请求中携带的响应 oneshot 发回.
//!
//! - 日志批发线程: 接收 `DbClient::log_append` 投递的日志, 缓存到内部缓冲区,
//!   并检查待写总条数与总大小限制. 收到日志写入线程的批发请求时, 把整批日志
//!   发过去.
//!
//! - 日志写入线程: 向日志批发线程请求一批日志, 并在单个 sqlite 事务内批量写入.
//!
//! tokio 的 async 线程不直接访问数据库, 因此不会被 sqlite IO 阻塞.
//!
//! ## 事务约定
//!
//! 每个 pub API 的单次操作 (打开 / 读 / 写) 都通过 `Db::with_tx` 处理:
//! 锁数据库连接一次, 创建事务一次, 全部 sqlite 读写都在该事务内完成.
//!
//! 唯一的例外是 `vacuum`, 因为 sqlite 不允许 `VACUUM` 在事务内执行.

mod client;
mod error;
mod logd;
mod rw;
mod schema;
mod util;

pub use client::DbClient;
pub use error::DbError;
pub use logd::{LogEntry, LogPageRaw, LogPos};
pub use schema::{PROGRAM_NAME, SCHEMA_VERSION};

/// 单个 unit 的持久状态 (unit 表中的一行)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitState {
    /// unit 名称
    pub name: String,
    /// 是否 enable
    pub enabled: bool,
    /// 是否 mask
    pub masked: bool,
}
