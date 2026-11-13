//! init 回复给 ctl / log 的响应消息 (结构化)

use serde::{Deserialize, Serialize};

use crate::{
    conf::{Unit, UnitType},
    sysd::{ServiceSta, UnitSta, UnitSubSta},
};

/// 一条日志记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdResLogEntry {
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

/// unit 的状态 (统一接口, 用于 list-units / unit-state)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdResUnitState {
    /// unit 名称
    pub name: String,
    /// 描述
    pub description: String,
    /// unit 类型
    pub unit_type: UnitType,
    /// 激活状态
    pub active: UnitSta,
    /// 子状态
    pub sub: UnitSubSta,
    /// 是否 enable
    pub enabled: bool,
    /// 是否 mask
    pub masked: bool,
}

/// unit 的详细状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdResUnitStatus {
    /// unit 名称
    pub name: String,
    /// 描述
    pub description: String,
    /// 是否 enable
    pub enabled: bool,
    /// 是否 mask
    pub masked: bool,
    /// 激活状态
    pub active: UnitSta,
    /// 按 unit 类型区分的 特有状态
    pub s: SdResUnitS,
}

/// unit 特有状态 (包装 service / timer)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "kebab-case")]
pub enum SdResUnitS {
    /// service 特有状态
    Service(SdResServiceStatus),
    /// timer 特有状态
    Timer(SdResTimerStatus),
}

/// service 特有状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdResServiceStatus {
    /// service 子状态
    pub sub: ServiceSta,
    /// 进程 pid (运行中)
    pub pid: Option<u32>,
    /// 自动重启次数
    pub restart_count: u32,
    /// 最后一次退出状态码
    pub exit_status: Option<i32>,
}

/// timer 特有状态 (预留, 目前为空)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdResTimerStatus {}

/// timer 信息 (初版为空, 保留)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdResTimerInfo {
    /// timer 名称
    pub name: String,
}

/// init 回复给 ctl 的响应 (结构化)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "res", content = "data", rename_all = "kebab-case")]
pub enum SdRes {
    /// 操作成功 (无需额外数据)
    Ok,
    /// 操作失败 (携带错误消息, 尽力而为)
    Err(String),
    /// `list-units` 结果
    ListUnits(Vec<SdResUnitState>),
    /// `status` 结果
    Status(SdResUnitStatus),
    /// unit 配置查询结果
    Config(Unit),
    /// unit 状态查询结果 (`None` 表示未找到)
    UnitState(Option<SdResUnitState>),
    /// `get-default` 结果
    DefaultTarget(String),
    /// `list-dependencies` 结果
    Dependencies(Vec<String>),
    /// `list-timers` 结果
    Timers(Vec<SdResTimerInfo>),
    /// 日志 查询结果 (分页)
    Logs {
        /// 匹配条件的日志总量
        total: u64,
        /// 下一页游标 (无更多日志时为空)
        cursor: Option<i64>,
        /// 本页日志列表
        logs: Vec<SdResLogEntry>,
    },
}
