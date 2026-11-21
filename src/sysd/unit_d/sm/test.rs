//! UnitD 状态机分支测试.

use crate::{
    conf::Conf,
    jsonl::{SdReq, SdReqUnit, SdReqUnitOp, SdReqUnitd, SdReqUnitdRO, SdRes},
    sysd::{TARGET_DEFAULT, reload::ReloadR},
    sysu::DaemonSta,
};

use super::super::unit_s::{UnitS, UnitSME};
use super::{Sm, SmInput, SmOutput};

fn empty_sm() -> Sm {
    Sm::new(UnitS::new(Vec::new(), UnitSME::default()), String::new())
}

#[test]
fn self_serial_ops_wait_while_busy() {
    let mut sm = empty_sm();

    // 第一次 daemon-reload 立即开始 (进入 busy).
    let actions = sm.t(SmInput::SdReq(SdReq::UnitD(SdReqUnitd::DaemonReload)));
    assert!(matches!(actions.as_slice(), [SmOutput::Reload(..)]));
    assert!(sm.is_busy());

    // busy 时, 自身串行操作返回 Wait, 不修改状态.
    let actions = sm.t(SmInput::SdReq(SdReq::UnitD(SdReqUnitd::DaemonReload)));
    assert!(matches!(actions.as_slice(), [SmOutput::Wait]));

    let actions = sm.t(SmInput::SdReq(SdReq::Unit(SdReqUnit {
        name: "t.service".to_string(),
        op: SdReqUnitOp::Enable { now: false },
    })));
    assert!(matches!(actions.as_slice(), [SmOutput::Wait]));

    // 转发 ServiceD 的操作不受 busy 限制, 直接转发.
    let actions = sm.t(SmInput::SdReq(SdReq::Unit(SdReqUnit {
        name: "t.service".to_string(),
        op: SdReqUnitOp::Start,
    })));
    assert!(matches!(actions.as_slice(), [SmOutput::SendService(..)]));
}

#[test]
fn immediate_self_serial_op_emits_idle() {
    let mut sm = empty_sm();

    let actions = sm.t(SmInput::SdReq(SdReq::UnitD(SdReqUnitd::SetDefault {
        target: TARGET_DEFAULT.to_string(),
    })));
    assert!(
        matches!(
            actions.as_slice(),
            [SmOutput::Reply(SdRes::Ok(None)), SmOutput::Idle]
        ),
        "unexpected actions: {:?}",
        actions
    );
    assert!(!sm.is_busy());
}

#[test]
fn async_self_serial_op_completion_emits_idle() {
    let mut sm = empty_sm();

    let actions = sm.t(SmInput::SdReq(SdReq::UnitD(SdReqUnitd::DaemonReload)));
    assert!(matches!(actions.as_slice(), [SmOutput::Reload(..)]));
    assert!(sm.is_busy());

    let load = ReloadR {
        conf: Conf {
            units: Vec::new(),
            db_path: String::new(),
            db_log_no_index: false,
            warnings: Vec::new(),
        },
        me: UnitSME::default(),
    };
    let actions = sm.t(SmInput::ReloadOk(load));
    assert!(!sm.is_busy());
    assert!(
        matches!(actions.last(), Some(SmOutput::Idle)),
        "expected Idle as last action: {:?}",
        actions
    );
}

#[test]
fn system_state_queries_while_running_and_stops_while_exiting() {
    let mut sm = empty_sm();

    let req = SdReq::UnitdRO(SdReqUnitdRO::SystemState);
    let actions = sm.t(SmInput::SdReq(req.clone()));
    assert!(
        matches!(actions.as_slice(), [SmOutput::QueryService(..)]),
        "unexpected actions: {:?}",
        actions
    );

    let actions = sm.t(SmInput::Exit);
    assert!(matches!(actions.as_slice(), [SmOutput::ExitService]));

    let actions = sm.t(SmInput::SdReq(req));
    assert!(
        matches!(
            actions.as_slice(),
            [SmOutput::Reply(SdRes::SystemState(DaemonSta::Stopping))]
        ),
        "unexpected actions: {:?}",
        actions
    );
}

#[test]
fn exiting_rejects_self_serial_ops() {
    let mut sm = empty_sm();

    let actions = sm.t(SmInput::Exit);
    assert!(matches!(actions.as_slice(), [SmOutput::ExitService]));

    let actions = sm.t(SmInput::SdReq(SdReq::UnitD(SdReqUnitd::DaemonReload)));
    assert!(
        matches!(&actions[0], SmOutput::Reply(SdRes::Err(m)) if m.contains("exiting")),
        "unexpected actions: {:?}",
        actions
    );

    let actions = sm.t(SmInput::SdReq(SdReq::Unit(SdReqUnit {
        name: "t.service".to_string(),
        op: SdReqUnitOp::Enable { now: false },
    })));
    assert!(
        matches!(&actions[0], SmOutput::Reply(SdRes::Err(m)) if m.contains("exiting")),
        "unexpected actions: {:?}",
        actions
    );
}
