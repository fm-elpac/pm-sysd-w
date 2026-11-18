//! init 回复给 ctl / log 的响应消息 (结构化)

use std::fmt::Debug;

use serde::{Deserialize, Serialize};

use crate::{
    conf::{Unit, UnitType},
    sysu::{EnableSta, LoadSta, UnitSta, UnitSubSta},
};

use super::SdResLogPage;

/// init 回复给 ctl 的响应 (结构化)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "res", content = "data", rename_all = "kebab-case")]
pub enum SdRes {
    /// 操作成功 (可选提示消息, 由 ctl 打印)
    Ok(Option<String>),
    /// 操作失败 (携带错误消息, 尽力而为)
    Err(String),
    /// (分页) 日志查询
    Logs(SdResLogPage),
    /// (分页) `list-units`
    ListUnits(SdResUnitPage),
    /// (分页) `list-timers`
    Timers(SdResTimerPage),
    /// (分页) `list-dependencies`
    Dependencies(SdResDepPage),
    /// unit 配置查询
    Config(Unit),
    /// unit 状态查询 (`None` 表示未找到)
    UnitState(Option<SdResUnitState>),
    /// `status`
    Status(SdResUnitStatus),
    /// `get-default`
    DefaultTarget(String),
}

/// 分页查询结果 (通用)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "T: Serialize, C: Serialize",
    deserialize = "T: Deserialize<'de>, C: Deserialize<'de>"
))]
pub struct SdResPage<T: Debug + Clone, C: Debug + Clone> {
    /// 匹配查询条件的数据总条数
    pub total: u64,
    /// 下一页游标 (`None` 表示没有更多数据)
    pub cursor: Option<C>,
    /// 本页数据
    pub list: Vec<T>,
}

/// `list-units` 分页查询结果
pub type SdResUnitPage = SdResPage<SdResUnitState, String>;

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
    /// 加载状态
    pub load: LoadSta,
    /// 启用状态
    pub enable: EnableSta,
}

/// `list-timers` 分页查询结果
pub type SdResTimerPage = SdResPage<SdResTimerInfo, String>;

/// timer 信息 (初版为空, 保留)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdResTimerInfo {
    /// timer 名称
    pub name: String,
}

/// `list-dependencies` 分页查询结果
pub type SdResDepPage = SdResPage<String, String>;

/// unit 的详细状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdResUnitStatus {
    /// unit 名称
    pub name: String,
    /// 描述
    pub description: String,
    /// 加载状态
    pub load: LoadSta,
    /// 启用状态
    pub enable: EnableSta,
    /// 激活状态
    pub active: UnitSta,
    /// 子状态
    pub sub: UnitSubSta,
    /// 当前状态进入时间 (Unix 毫秒时间戳, 无状态变化时为 None)
    pub since: Option<i64>,
    /// 按 unit 类型区分的特有状态
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
