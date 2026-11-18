//! ServiceD 状态机分支测试.

use crate::{
    conf::{Install, RestartPolicy, Service, ServiceType, Unit, UnitType, Win},
    jsonl::{SdReqUnitOp, SdRes},
    sysd::service_s::ServiceStatus,
};

use super::super::ServiceCmdUnit;
use super::{ServiceOp, Sm, SmInput, SmOutput};

/// 构造一个最小 service 配置 (用于纯状态机测试).
fn service_unit(name: &str) -> Unit {
    Unit {
        name: name.to_string(),
        unit_type: UnitType::Service,
        description: String::new(),
        start_limit_interval_sec: 0,
        start_limit_burst: 0,
        service: Service {
            service_type: ServiceType::Simple,
            exec_start: vec![vec!["true".to_string()]],
            exec_stop: Vec::new(),
            exec_reload: Vec::new(),
            restart: RestartPolicy::No,
            restart_sec: 0,
            timeout_start_sec: 0,
            timeout_stop_sec: 0,
            working_directory: None,
            environment: Vec::new(),
            remain_after_exit: false,
        },
        install: Install {
            wanted_by: Vec::new(),
        },
        win: Win {
            create_no_window: false,
        },
    }
}

/// 构造单个 unit 操作.
fn unit_cmd(name: &str, op: SdReqUnitOp, conf: Option<Unit>) -> ServiceCmdUnit {
    ServiceCmdUnit {
        name: name.to_string(),
        conf,
        op,
    }
}

#[test]
fn sm_start_creates_new_service() {
    let (s, out) = Sm::new().t(SmInput::Unit(unit_cmd(
        "t.service",
        SdReqUnitOp::Start,
        Some(service_unit("t.service")),
    )));
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], SmOutput::StartService { name, .. } if name == "t.service"));
    assert!(s.running_unit("t.service").is_none());
}

#[test]
fn sm_start_active_sends_command() {
    let sm = Sm::new();
    let (sm, out) = sm.t(SmInput::ServiceStarted {
        name: "t.service".to_string(),
        conf: service_unit("t.service"),
    });
    assert!(out.is_empty());

    let (_, out) = sm.t(SmInput::Unit(unit_cmd(
        "t.service",
        SdReqUnitOp::Start,
        Some(service_unit("t.service")),
    )));
    assert_eq!(out.len(), 1);
    assert!(
        matches!(&out[0], SmOutput::SendServiceS { name, op: ServiceOp::Start } if name == "t.service")
    );
}

#[test]
fn sm_stop_unknown_replies_not_found() {
    let (_, out) = Sm::new().t(SmInput::Unit(unit_cmd(
        "nope.service",
        SdReqUnitOp::Stop,
        None,
    )));
    assert_eq!(out.len(), 1);
    assert!(
        matches!(&out[0], SmOutput::Reply(SdRes::Err(m)) if m == "Unit nope.service not found.")
    );
}

#[test]
fn sm_start_many_mixes_active_and_new() {
    let sm = Sm::new();
    let (sm, _) = sm.t(SmInput::ServiceStarted {
        name: "a.service".to_string(),
        conf: service_unit("a.service"),
    });

    let (_, out) = sm.t(SmInput::StartMany(vec![
        ("a.service".to_string(), service_unit("a.service")),
        ("b.service".to_string(), service_unit("b.service")),
    ]));
    assert_eq!(out.len(), 3);
    assert!(
        matches!(&out[0], SmOutput::SendServiceS { name, op: ServiceOp::Start } if name == "a.service")
    );
    assert!(matches!(&out[1], SmOutput::StartService { name, .. } if name == "b.service"));
    assert!(matches!(&out[2], SmOutput::ReplyResult(Ok(()))));
}

#[test]
fn sm_exit_requests_stop_all() {
    let (_, out) = Sm::new().t(SmInput::Exit);
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], SmOutput::StopAll));
}

#[test]
fn sm_stop_all_done_replies_and_exits() {
    let sm = Sm::new();
    let (sm, _) = sm.t(SmInput::Exit);

    let (_, out) = sm.t(SmInput::StopAllDone(Ok(())));
    assert_eq!(out.len(), 2);
    assert!(matches!(&out[0], SmOutput::ReplyResult(Ok(()))));
    assert!(matches!(&out[1], SmOutput::Exit));
}

#[test]
fn sm_stopping_rejects_unit() {
    let sm = Sm::new();
    let (sm, _) = sm.t(SmInput::Exit);

    let (_, out) = sm.t(SmInput::Unit(unit_cmd(
        "t.service",
        SdReqUnitOp::Start,
        Some(service_unit("t.service")),
    )));
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], SmOutput::Reply(SdRes::Err(m)) if m == "service-d exiting"));
}

#[test]
fn sm_stopping_rejects_start_many() {
    let sm = Sm::new();
    let (sm, _) = sm.t(SmInput::Exit);

    let (_, out) = sm.t(SmInput::StartMany(vec![(
        "t.service".to_string(),
        service_unit("t.service"),
    )]));
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], SmOutput::ReplyResult(Err(_))));
}

#[test]
fn sm_stopping_rejects_exit() {
    let sm = Sm::new();
    let (sm, _) = sm.t(SmInput::Exit);

    let (_, out) = sm.t(SmInput::Exit);
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], SmOutput::ReplyResult(Err(_))));
}

#[test]
fn sm_terminated_records_last_status() {
    let sm = Sm::new();
    let (sm, _) = sm.t(SmInput::ServiceStarted {
        name: "t.service".to_string(),
        conf: service_unit("t.service"),
    });

    let status = ServiceStatus {
        failed: true,
        restart_count: 2,
        exit_status: Some(1),
        since: Some(123_456),
    };
    let (sm, out) = sm.t(SmInput::ServiceTerminated {
        name: "t.service".to_string(),
        status,
    });
    assert!(out.is_empty());
    assert!(sm.running_unit("t.service").is_none());

    let last = sm.last_status("t.service").unwrap();
    assert!(last.failed);
    assert_eq!(last.restart_count, 2);
    assert_eq!(last.exit_status, Some(1));
    assert_eq!(last.since, Some(123_456));
}
