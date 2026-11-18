//! SysD 状态机 (纯计算).

use crate::{
    jsonl::{SdReq, SdReqSysd, SdRes},
    sysd::{SysdError, TARGET_EXIT, init::Shutdown},
    sysu::reached_target,
};

use log::trace;
use serde::Serialize;

mod io;

pub use io::{SmInput, SmOutput};

/// 状态机的状态
#[derive(Debug, Clone, Serialize)]
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

impl Sm {
    /// 初始状态
    pub fn new() -> Self {
        Self::Running
    }

    /// 状态机主要处理逻辑 (入口)
    pub fn t(self, input: SmInput) -> (Self, Vec<SmOutput>) {
        trace!("in {:?}", input);
        let (s, out) = self.it(input);
        trace!("out {:?}", out);
        (s, out)
    }

    /// 状态机主要处理逻辑 (内部实现, 由 [`Sm::t`] 包装 trace)
    fn it(self, input: SmInput) -> (Self, Vec<SmOutput>) {
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
            } else {
                // 模仿 systemd: 成功停止所有 service 后, 输出到达 exit.target.
                a.push(SmOutput::Console {
                    msg: reached_target(TARGET_EXIT),
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

#[cfg(test)]
mod test;
