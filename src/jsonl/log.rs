//! 日志分页查询.

use serde::{Deserialize, Serialize};

use super::{SdReqPagePos, SdResPage};

/// 日志只读查询请求
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum SdReqLogRO {
    /// 查询日志
    Query(SdReqLogQuery),
}

/// 日志分页查询条件.
///
/// 比如, 查询最新的 1000 条日志, 升序排序, 应该使用:
///
/// ```rust,ignore
/// SdReqLogQuery {
///     filter: SdReqLogQueryFilter {
///         level: None,
///         unit: None,
///     },
///     // 从最新数据向前跳过 999 条, 即从第 1000 条最新数据开始.
///     pos: SdReqPagePos::End(999),
///     asc: true,
///     limit: 1000,
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdReqLogQuery {
    /// 过滤条件
    pub filter: SdReqLogQueryFilter,
    /// 开始查询位置
    pub pos: SdReqPagePos<i64>,
    /// 查询方向 (升序).
    ///
    /// - `false`: 结果降序排序 (最新数据在前).
    /// - `true`: 结果升序排序 (最旧数据在前).
    pub asc: bool,
    /// 此页最多读取条数
    pub limit: u32,
}

/// 日志查询过滤条件: 所有条件必须同时满足, 省略某条件则不按其过滤
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdReqLogQueryFilter {
    /// 按 level 过滤
    #[serde(default)]
    pub level: Option<String>,
    /// 按 unit 过滤
    #[serde(default)]
    pub unit: Option<String>,
}

/// 日志分页查询结果
pub type SdResLogPage = SdResPage<SdResLogItem, i64>;

/// 一条日志记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdResLogItem {
    /// 自增 id
    pub id: i64,
    /// 时间戳 (UTC, ISO 8601)
    pub ts: String,
    /// 日志级别
    pub level: String,
    /// 关联 unit 名 (可为空)
    pub unit: Option<String>,
    /// 日志内容
    pub msg: String,
}
