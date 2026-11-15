//! SysD 状态机 (纯计算).

use crate::{
    jsonl::{SdReq, SdReqLogRO, SdReqSysd, SdRes},
    sysd::SysdError,
};

use super::super::init::Shutdown;

/// 状态机的状态
pub enum Sm {
    /// 正常运行 (接受修改请求)
    Running,
    /// 已请求 UnitD 停止所有 unit, 等待其完成 (退出序列第一步)
    StoppingUnits {
        /// 退出方式 (决定通知 init 时发送的值)
        shutdown: Shutdown,
    },
    /// 退出序列最后一步 (actor 收到 [`SmOutput::Finish`] 后立即退出)
    ShuttingDown,
}

/// 状态机输入数据 (事件)
pub enum SmInput {
    /// 处理一条 ctl 请求
    SdReq(SdReq),
    /// 启动默认 target
    StartDefaultTarget,
    /// UnitD 已停止所有 unit (退出序列内部事件)
    ExitUnitsDone(Result<(), SysdError>),
}

/// 状态机输出数据 (操作 / IO 请求)
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
    /// 通知 `sysd::init` 退出, 并退出 actor 主循环 (退出序列最后一步)
    Finish {
        /// 退出方式
        shutdown: Shutdown,
    },
}

impl Sm {
    /// 初始状态
    pub fn new() -> Self {
        Self::Running
    }

    /// 状态机主要处理逻辑 (入口)
    pub fn t(self, input: SmInput) -> (Self, Vec<SmOutput>) {
        match self {
            Self::Running => running(input),
            Self::StoppingUnits { shutdown } => stopping_units(input, shutdown),
            Self::ShuttingDown => unreachable!("SysD state machine already shutting down"),
        }
    }
}

/// 正常运行状态下处理输入
fn running(input: SmInput) -> (Sm, Vec<SmOutput>) {
    match input {
        SmInput::SdReq(SdReq::SysD(SdReqSysd::SqliteVacuum)) => {
            (Sm::Running, vec![SmOutput::SqliteVacuum])
        }
        SmInput::SdReq(SdReq::SysD(SdReqSysd::SoftReboot)) => {
            exit_start("info", "soft-reboot requested", Shutdown::SoftReboot)
        }
        SmInput::SdReq(SdReq::SysD(SdReqSysd::Exit)) => {
            exit_start("info", "exit requested", Shutdown::Exit)
        }
        SmInput::SdReq(SdReq::LogRO(log_req)) => (Sm::Running, vec![SmOutput::SendLog(log_req)]),
        SmInput::SdReq(
            req @ (SdReq::UnitdRO(_) | SdReq::UnitRO(_) | SdReq::UnitD(_) | SdReq::Unit(_)),
        ) => (Sm::Running, vec![SmOutput::SendUnitd(req)]),
        SmInput::StartDefaultTarget => (Sm::Running, vec![SmOutput::SendStartDefaultTarget]),
        _ => unreachable!("unexpected SysD input while running"),
    }
}

/// 已请求 UnitD 停止所有 unit, 等待其完成
fn stopping_units(input: SmInput, shutdown: Shutdown) -> (Sm, Vec<SmOutput>) {
    match input {
        SmInput::ExitUnitsDone(result) => {
            let mut a = Vec::new();
            if let Err(e) = result {
                a.push(SmOutput::Log {
                    level: "error",
                    msg: format!("stop all units: {}", e),
                });
            }
            // 先回复调用方, 再通知 init 并退出 actor
            a.push(SmOutput::Reply(SdRes::Ok(None)));
            a.push(SmOutput::Finish { shutdown });
            (Sm::ShuttingDown, a)
        }
        SmInput::SdReq(SdReq::SysD(_) | SdReq::UnitD(_) | SdReq::Unit(_)) => (
            Sm::StoppingUnits { shutdown },
            vec![SmOutput::Reply(SdRes::Err("sysd exiting".to_string()))],
        ),
        SmInput::SdReq(SdReq::LogRO(log_req)) => (
            Sm::StoppingUnits { shutdown },
            vec![SmOutput::SendLog(log_req)],
        ),
        SmInput::SdReq(req @ (SdReq::UnitdRO(_) | SdReq::UnitRO(_))) => (
            Sm::StoppingUnits { shutdown },
            vec![SmOutput::SendUnitd(req)],
        ),
        SmInput::StartDefaultTarget => (
            Sm::StoppingUnits { shutdown },
            vec![SmOutput::ReplyResult(Err(SysdError::Failed(
                "sysd exiting".to_string(),
            )))],
        ),
    }
}

/// 构造 exit / soft-reboot 的第一阶段动作 (记录日志并请求 UnitD 停止)
fn exit_start(level: &'static str, msg: &'static str, shutdown: Shutdown) -> (Sm, Vec<SmOutput>) {
    (
        Sm::StoppingUnits { shutdown },
        vec![
            SmOutput::Log {
                level,
                msg: msg.to_string(),
            },
            SmOutput::SendExitUnitd,
        ],
    )
}
