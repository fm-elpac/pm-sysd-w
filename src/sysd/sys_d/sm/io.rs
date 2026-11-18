//! SysD 状态机输入输出类型.

use serde::Serialize;

use crate::{
    jsonl::{SdReq, SdReqLogRO, SdRes},
    sysd::{SysdError, init::Shutdown},
};

/// 状态机输入数据 (事件)
#[derive(Debug, Clone, Serialize)]
pub enum SmInput {
    /// 处理一条 ctl 请求
    SdReq(SdReq),
    /// 启动默认 target
    StartDefaultTarget,
    /// UnitD 已停止所有 unit (退出序列内部事件)
    ExitUnitsDone(Result<(), SysdError>),
}

/// 状态机输出数据 (操作 / IO 请求)
#[derive(Debug, Clone, Serialize)]
pub enum SmOutput {
    /// 写一条日志
    Log { level: &'static str, msg: String },
    /// 回复 `SdReq` 调用方
    Reply(SdRes),
    /// 回复 `StartDefaultTarget` 调用方
    ReplyResult(Result<(), SysdError>),
    /// 发送日志只读查询给 Logd
    SendLog(SdReqLogRO),
    /// 发送 ctl 请求给 UnitD
    SendUnitd(SdReq),
    /// 发送启动默认 target 给 UnitD
    SendStartDefaultTarget,
    /// 对数据库执行 vacuum
    SqliteVacuum,
    /// 发送退出请求给 UnitD (退出序列第一步)
    SendExitUnitd,
    /// 向控制台输出一行消息 (systemd 风格的启动/退出状态行)
    Console {
        /// 完整消息行 (不含换行)
        msg: String,
    },
    /// 通知 `sysd::init` 退出, 并退出 actor 主循环 (退出序列最后一步)
    Finish {
        /// 退出方式
        shutdown: Shutdown,
    },
}
