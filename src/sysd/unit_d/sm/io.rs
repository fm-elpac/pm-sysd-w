//! UnitD 状态机输入输出类型.

use serde::Serialize;

use crate::{
    conf::Unit,
    jsonl::{SdReq, SdRes},
    sysd::{SysdError, reload::ReloadR, service_d::ServiceCmdUnit},
};

use super::super::query::QueryReq;

/// 状态机输入数据 (事件)
#[derive(Debug, Clone, Serialize)]
pub enum SmInput {
    /// 处理一条 ctl 请求
    SdReq(SdReq),
    /// 启动默认 target 中已 enable 的 service
    StartDefaultTarget,
    /// 停止所有 unit, 然后退出 actor 主循环
    Exit,
    /// daemon-reload 加载完成 (携带完整加载结果)
    ReloadOk(ReloadR),
    /// daemon-reload 加载失败 (携带错误消息)
    ReloadErr(String),
    /// 数据库写入完成 (携带结果与对应的后续操作)
    WriteDbDone(Result<(), SysdError>, PendingWrite),
    /// ServiceD 已停止所有 service (退出序列内部事件)
    ExitServiceDone(Result<(), SysdError>),
}

/// 状态机输出的动作 (由 UnitD actor 执行)
#[derive(Debug, Clone, Serialize)]
pub enum SmOutput {
    /// 当前自身串行操作不能立即处理, 需要放入 Actor 的等候队列
    Wait,
    /// 状态机已完成一个自身串行操作, Actor 可以喂入等候队列的下一个操作
    Idle,
    /// 写一条日志 (调用 logd)
    Log {
        level: &'static str,
        unit: Option<String>,
        msg: String,
    },
    /// 立即返回 SdReq 响应
    Reply(SdRes),
    /// 立即返回 StartDefaultTarget / Exit 响应
    ReplyResult(Result<(), SysdError>),
    /// 给自己发送进一步操作
    SendSelf(SdReq),
    /// 转发单个 unit 操作给 ServiceD
    SendService(ServiceCmdUnit),
    /// 批量启动 (转发给 ServiceD)
    SendStartMany(Vec<(String, Unit)>),
    // 转发命令给 TimerD (预留)
    //SendTimer,
    /// 查询 (组合 UnitD 与 ServiceD 数据), 完成后回复 SdReq 调用方
    QueryService(QueryReq),
    /// 写数据库 (mask / enable), 携带完成后需要执行的后续操作, 由后台 task 回传
    WriteDb(WriteUnit, PendingWrite),
    /// daemon-reload (携带配置文件路径)
    Reload(String),
    /// 向 ServiceD 发送退出消息 (ServiceD 停止所有 service 并退出)
    ExitService,
    /// 退出 UnitD 主循环
    Exit,
}

/// 向数据库写入一个 unit 的完整持久状态
#[derive(Debug, Clone, Serialize)]
pub struct WriteUnit {
    pub name: String,
    pub enabled: bool,
    pub masked: bool,
}

/// 等待数据库写入完成后需要执行的后续操作
#[derive(Debug, Clone, Serialize)]
pub enum PendingWrite {
    /// enable (now 表示 --now)
    Enable { name: String, now: bool },
    /// disable (now 表示 --now)
    Disable { name: String, now: bool },
    /// reenable
    Reenable { name: String },
    /// mask (stop 表示 --now 且 unit 存在; warning 为 unit 不存在时的警告)
    Mask {
        name: String,
        stop: bool,
        warning: Option<String>,
    },
    /// unmask
    Unmask,
}
