//! ctl / log 发给 init 的请求消息 (结构化).

use serde::{Deserialize, Serialize};

/// ctl 发给 init 的请求.
///
/// 外层按处理组件与是否只读分组 (`req` 字段), 内层为各组的具体操作
/// (`op` 字段).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "req", content = "data", rename_all = "kebab-case")]
pub enum SdReq {
    /// 由 sysd 模块 SysD 组件处理 (soft-reboot / exit)
    SysD(SdReqSysd),
    /// logd 只读查询
    LogRO(SdReqLogRO),
    /// UnitD 只读查询
    UnitdRO(SdReqUnitdRO),
    /// UnitD 修改操作
    UnitD(SdReqUnitd),
    /// 单个 unit 只读查询
    UnitRO(SdReqUnitRO),
    /// 单个 unit 修改操作
    Unit(SdReqUnit),
}

/// SysD 组件直接处理的请求
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum SdReqSysd {
    /// 重启 init 及所有 service
    SoftReboot,
    /// 退出 init
    Exit,
}

/// 日志 只读查询请求
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum SdReqLogRO {
    /// 读取日志
    Query(SdReqLogQuery),
}

/// 日志查询条件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdReqLogQuery {
    /// 按 unit 过滤 (省略则不过滤)
    pub unit: Option<String>,
    /// 每页最多读取条数
    #[serde(default)]
    pub limit: u32,
    /// 分页游标 (`Some(id)` 时读取 `id < cursor` 的日志; 省略, 则读取最新).
    #[serde(default)]
    pub cursor: Option<i64>,
}

/// UnitD 只读查询 (不针对单个 unit)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum SdReqUnitdRO {
    /// 读取默认 target
    GetDefault,
    /// 列出 unit
    ListUnits,
    /// 列出 timer
    ListTimers,
    /// 显示环境变量 (初版 TODO)
    ShowEnvironment,
}

/// UnitD 修改操作 (不针对单个 unit).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum SdReqUnitd {
    /// 设置默认 target
    SetDefault {
        /// target 名称.
        target: String,
    },
    /// 设置 环境变量 (初版 TODO)
    SetEnvironment,
    /// 删除 环境变量 (初版 TODO)
    UnsetEnvironment,
    /// 重新加载配置文件
    DaemonReload,
}

/// 单个 unit 只读查询
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdReqUnitRO {
    /// 要查询的 unit 名称
    pub name: String,
    /// 具体查询操作
    #[serde(flatten)]
    pub op: SdReqUnitOpRO,
}

/// 单个 unit 只读查询具体操作
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum SdReqUnitOpRO {
    /// 查询 unit 状态 (统一接口)
    UnitState,
    /// 查询 unit 详细状态
    Status,
    /// 读取 unit 配置
    Config,
    /// 列出依赖
    ListDependencies,
}

/// 单个 unit 修改操作
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdReqUnit {
    /// 要操作的 unit 名称 (不指定 unit 时传空字符串 "").
    pub name: String,
    /// 具体操作 (可携带额外参数)
    #[serde(flatten)]
    pub op: SdReqUnitOp,
}

/// 单个 unit 修改操作 (具体)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum SdReqUnitOp {
    /// enable
    Enable {
        /// 是否同时 start
        #[serde(default)]
        now: bool,
    },
    /// disable
    Disable {
        /// 是否同时 stop
        #[serde(default)]
        now: bool,
    },
    /// reenable
    Reenable,
    /// preset (初版 TODO)
    Preset,
    /// mask
    Mask,
    /// unmask
    Unmask,
    /// 启动
    Start,
    /// 停止
    Stop,
    /// reload
    Reload,
    /// 重启
    Restart,
    /// 仅 active 时重启
    TryRestart,
    /// 有 ExecReload 则 reload, 否则 restart.
    ReloadOrRestart,
    /// 仅 active 时 reload-or-restart
    TryReloadOrRestart,
    /// 强制终止
    Kill,
    /// 清理状态
    Clean,
    /// 清除 failed 状态 (`name` 为空则清除全部)
    ResetFailed,
}
