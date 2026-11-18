//! SysD 状态机分支测试.

use crate::{
    jsonl::{
        SdReq, SdReqLogQuery, SdReqLogQueryFilter, SdReqLogRO, SdReqPagePos, SdReqSysd, SdReqUnit,
        SdReqUnitList, SdReqUnitOp, SdReqUnitOpRO, SdReqUnitRO, SdReqUnitd, SdReqUnitdRO, SdRes,
    },
    sysd::{SysdError, init::Shutdown},
};

use super::{Sm, SmInput, SmOutput};

/// 在 Running 状态处理一次输入.
fn sm_running(input: SmInput) -> (Sm, Vec<SmOutput>) {
    Sm::new().t(input)
}

/// 在 StoppingUnits 状态 (Exit) 处理一次输入.
fn sm_stopping(input: SmInput) -> (Sm, Vec<SmOutput>) {
    Sm::StoppingUnits {
        shutdown: Shutdown::Exit,
    }
    .t(input)
}

#[test]
fn sm_initial_running() {
    assert!(matches!(Sm::new(), Sm::Running));
}

#[test]
fn sm_running_vacuum() {
    let (s, out) = sm_running(SmInput::SdReq(SdReq::SysD(SdReqSysd::SqliteVacuum)));
    assert!(matches!(s, Sm::Running));
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], SmOutput::SqliteVacuum));
}

#[test]
fn sm_running_exit() {
    let (s, out) = sm_running(SmInput::SdReq(SdReq::SysD(SdReqSysd::Exit)));
    assert!(matches!(
        s,
        Sm::StoppingUnits {
            shutdown: Shutdown::Exit
        }
    ));
    assert_eq!(out.len(), 2);
    assert!(matches!(&out[0], SmOutput::Log { level: "info", msg } if msg == "exit requested"));
    assert!(matches!(&out[1], SmOutput::SendExitUnitd));
}

#[test]
fn sm_running_soft_reboot() {
    let (s, out) = sm_running(SmInput::SdReq(SdReq::SysD(SdReqSysd::SoftReboot)));
    assert!(matches!(
        s,
        Sm::StoppingUnits {
            shutdown: Shutdown::SoftReboot
        }
    ));
    assert_eq!(out.len(), 2);
    assert!(
        matches!(&out[0], SmOutput::Log { level: "info", msg } if msg == "soft-reboot requested")
    );
    assert!(matches!(&out[1], SmOutput::SendExitUnitd));
}

#[test]
fn sm_running_forwards_log_ro() {
    let (s, out) = sm_running(SmInput::SdReq(SdReq::LogRO(SdReqLogRO::Query(
        SdReqLogQuery {
            filter: SdReqLogQueryFilter {
                level: None,
                unit: None,
            },
            pos: SdReqPagePos::End(0),
            asc: false,
            limit: 10,
        },
    ))));
    assert!(matches!(s, Sm::Running));
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], SmOutput::SendLog(_)));
}

#[test]
fn sm_running_forwards_unit_requests() {
    let reqs = [
        SdReq::UnitdRO(SdReqUnitdRO::ListUnits(SdReqUnitList {
            limit: 0,
            cursor: None,
        })),
        SdReq::UnitRO(SdReqUnitRO {
            name: "t.service".to_string(),
            op: SdReqUnitOpRO::Status,
        }),
        SdReq::UnitD(SdReqUnitd::DaemonReload),
        SdReq::Unit(SdReqUnit {
            name: "t.service".to_string(),
            op: SdReqUnitOp::Start,
        }),
    ];
    for req in reqs {
        let (s, out) = sm_running(SmInput::SdReq(req));
        assert!(matches!(s, Sm::Running));
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], SmOutput::SendUnitd(_)));
    }
}

#[test]
fn sm_running_start_default_target() {
    let (s, out) = sm_running(SmInput::StartDefaultTarget);
    assert!(matches!(s, Sm::Running));
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], SmOutput::SendStartDefaultTarget));
}

#[test]
fn sm_stopping_exit_units_done_ok() {
    let (s, out) = sm_stopping(SmInput::ExitUnitsDone(Ok(())));
    assert!(matches!(s, Sm::ShuttingDown));
    assert_eq!(out.len(), 3);
    assert!(
        matches!(&out[0], SmOutput::Console { msg } if msg == "[  OK  ] Reached target exit.target.")
    );
    assert!(matches!(&out[1], SmOutput::Reply(SdRes::Ok(None))));
    assert!(matches!(
        &out[2],
        SmOutput::Finish {
            shutdown: Shutdown::Exit
        }
    ));
}

#[test]
fn sm_stopping_exit_units_done_err() {
    let (s, out) = sm_stopping(SmInput::ExitUnitsDone(Err(SysdError::Failed(
        "boom".to_string(),
    ))));
    assert!(matches!(s, Sm::ShuttingDown));
    assert_eq!(out.len(), 3);
    assert!(
        matches!(&out[0], SmOutput::Log { level: "error", msg } if msg == "stop all units: sysd: boom")
    );
    assert!(matches!(&out[1], SmOutput::Reply(SdRes::Ok(None))));
    assert!(matches!(
        &out[2],
        SmOutput::Finish {
            shutdown: Shutdown::Exit
        }
    ));
}

#[test]
fn sm_stopping_rejects_modifying_requests() {
    let reqs = [
        SdReq::SysD(SdReqSysd::Exit),
        SdReq::UnitD(SdReqUnitd::DaemonReload),
        SdReq::Unit(SdReqUnit {
            name: "t.service".to_string(),
            op: SdReqUnitOp::Start,
        }),
    ];
    for req in reqs {
        let (s, out) = sm_stopping(SmInput::SdReq(req));
        assert!(matches!(s, Sm::StoppingUnits { .. }));
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], SmOutput::Reply(SdRes::Err(m)) if m == "sysd exiting"));
    }
}

#[test]
fn sm_stopping_forwards_log_ro() {
    let (s, out) = sm_stopping(SmInput::SdReq(SdReq::LogRO(SdReqLogRO::Query(
        SdReqLogQuery {
            filter: SdReqLogQueryFilter {
                level: None,
                unit: None,
            },
            pos: SdReqPagePos::End(0),
            asc: false,
            limit: 10,
        },
    ))));
    assert!(matches!(s, Sm::StoppingUnits { .. }));
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], SmOutput::SendLog(_)));
}

#[test]
fn sm_stopping_forwards_readonly_unit_requests() {
    let reqs = [
        SdReq::UnitdRO(SdReqUnitdRO::ListUnits(SdReqUnitList {
            limit: 0,
            cursor: None,
        })),
        SdReq::UnitRO(SdReqUnitRO {
            name: "t.service".to_string(),
            op: SdReqUnitOpRO::Status,
        }),
    ];
    for req in reqs {
        let (s, out) = sm_stopping(SmInput::SdReq(req));
        assert!(matches!(s, Sm::StoppingUnits { .. }));
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], SmOutput::SendUnitd(_)));
    }
}

#[test]
fn sm_stopping_rejects_start_default_target() {
    let (s, out) = sm_stopping(SmInput::StartDefaultTarget);
    assert!(matches!(s, Sm::StoppingUnits { .. }));
    assert_eq!(out.len(), 1);
    assert!(
        matches!(&out[0], SmOutput::ReplyResult(Err(SysdError::Failed(m))) if m == "sysd exiting")
    );
}
