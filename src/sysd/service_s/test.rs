//! service_s 模块测试.

use crate::conf::{Install, RestartPolicy, Service, ServiceType, Unit, UnitType, Win};

use super::super::service_d::ServiceOp;
use super::actor::{Action, Event, ExitInfo, ServiceSSM};

/// 构造一个最小可用的 service unit 配置.
fn test_unit(restart: RestartPolicy) -> Unit {
    Unit {
        name: "t.service".to_string(),
        unit_type: UnitType::Service,
        description: "test".to_string(),
        start_limit_interval_sec: 10,
        start_limit_burst: 5,
        service: Service {
            service_type: ServiceType::Simple,
            exec_start: vec!["true".to_string()],
            exec_stop: None,
            exec_reload: None,
            restart,
            restart_sec: 1,
            timeout_start_sec: 90,
            timeout_stop_sec: 90,
            working_directory: None,
            environment: vec![],
            remain_after_exit: false,
        },
        install: Install { wanted_by: None },
        win: Win {
            create_no_window: false,
        },
    }
}

#[test]
fn start_and_child_exit_no_restart() {
    let mut a = ServiceSSM::new(test_unit(RestartPolicy::No));
    let actions = a.handle(Event::Cmd(ServiceOp::Start)).unwrap();
    assert!(matches!(actions.as_slice(), [Action::SpawnChild]));
    assert!(a.is_active());
    assert!(a.child_running());

    let actions = a
        .handle(Event::ChildExited(ExitInfo {
            success: true,
            code: Some(0),
        }))
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [Action::NotifyTerminated { .. }]
    ));
    assert!(!a.is_active());
    assert!(!a.is_failed());
}

#[test]
fn child_exit_failure_schedules_restart() {
    let mut a = ServiceSSM::new(test_unit(RestartPolicy::OnFailure));
    a.handle(Event::Cmd(ServiceOp::Start)).unwrap();
    let actions = a
        .handle(Event::ChildExited(ExitInfo {
            success: false,
            code: Some(1),
        }))
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [Action::ScheduleRestart { .. }]
    ));
    assert!(a.restart_scheduled());
    assert!(!a.is_failed());
    assert_eq!(a.restart_count(), 1);
}

#[test]
fn stop_terminates() {
    let mut a = ServiceSSM::new(test_unit(RestartPolicy::No));
    a.handle(Event::Cmd(ServiceOp::Start)).unwrap();
    let actions = a.handle(Event::Cmd(ServiceOp::Stop)).unwrap();
    assert!(matches!(actions.as_slice(), [Action::KillChild]));
    assert!(!a.is_active());

    let actions = a
        .handle(Event::ChildExited(ExitInfo {
            success: true,
            code: Some(0),
        }))
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [Action::RunExecStop, Action::NotifyTerminated { .. }]
    ));
}
