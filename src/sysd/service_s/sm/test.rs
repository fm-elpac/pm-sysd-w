//! ServiceS 状态机单元测试.

use crate::{
    conf::{Install, RestartPolicy, Service, ServiceType, Unit, UnitType, Win},
    sysd::service_d::ServiceOp,
};

use super::{ExitInfo, Sm, SmInput, SmOutput};

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
            exec_start: vec![vec!["true".to_string()]],
            exec_stop: Vec::new(),
            exec_reload: Vec::new(),
            restart,
            restart_sec: 1,
            timeout_start_sec: 90,
            timeout_stop_sec: 90,
            working_directory: None,
            environment: vec![],
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

#[test]
fn start_and_child_exit_no_restart() {
    let mut a = Sm::new(test_unit(RestartPolicy::No));

    let (s1, actions) = a.t(SmInput::Cmd(ServiceOp::Start));
    a = s1;
    assert!(matches!(actions.as_slice(), [SmOutput::SpawnChild]));

    // simple 服务: spawn 成功即命令完成.
    let (s1, actions) = a.t(SmInput::Spawned { pid: Some(123) });
    a = s1;
    assert!(matches!(actions.as_slice(), [SmOutput::ReplyCmd(Ok(()))]));

    let (_s1, actions) = a.t(SmInput::ChildExited(ExitInfo {
        success: true,
        code: Some(0),
    }));
    assert!(matches!(actions.as_slice(), [SmOutput::RequestExit { .. }]));
}

#[test]
fn child_exit_failure_schedules_restart() {
    let mut a = Sm::new(test_unit(RestartPolicy::OnFailure));
    let (s1, _) = a.t(SmInput::Cmd(ServiceOp::Start));
    a = s1;
    let (s1, _) = a.t(SmInput::Spawned { pid: Some(123) });
    a = s1;

    let (_s1, actions) = a.t(SmInput::ChildExited(ExitInfo {
        success: false,
        code: Some(1),
    }));
    assert!(matches!(
        actions.as_slice(),
        [SmOutput::ScheduleRestart { .. }]
    ));
}

#[test]
fn stop_terminates() {
    let mut a = Sm::new(test_unit(RestartPolicy::No));
    let (s1, _) = a.t(SmInput::Cmd(ServiceOp::Start));
    a = s1;
    let (s1, _) = a.t(SmInput::Spawned { pid: Some(123) });
    a = s1;

    let (s1, actions) = a.t(SmInput::Cmd(ServiceOp::Stop));
    a = s1;
    assert!(matches!(actions.as_slice(), [SmOutput::KillChild]));

    let (_s1, actions) = a.t(SmInput::ChildExited(ExitInfo {
        success: true,
        code: Some(0),
    }));
    assert!(matches!(
        actions.as_slice(),
        [SmOutput::ReplyCmd(Ok(())), SmOutput::RequestExit { .. }]
    ));
}

#[test]
fn stop_runs_exec_stop_before_kill() {
    let mut conf = test_unit(RestartPolicy::No);
    conf.service.exec_stop = vec![vec!["true".to_string()]];
    let mut a = Sm::new(conf);
    let (s1, _) = a.t(SmInput::Cmd(ServiceOp::Start));
    a = s1;
    let (s1, _) = a.t(SmInput::Spawned { pid: Some(123) });
    a = s1;

    let (s1, actions) = a.t(SmInput::Cmd(ServiceOp::Stop));
    a = s1;
    assert!(matches!(actions.as_slice(), [SmOutput::RunExecStop]));

    let (s1, actions) = a.t(SmInput::ExecStopDone);
    a = s1;
    assert!(matches!(actions.as_slice(), [SmOutput::KillChild]));

    let (_s1, actions) = a.t(SmInput::ChildExited(ExitInfo {
        success: true,
        code: Some(0),
    }));
    assert!(matches!(
        actions.as_slice(),
        [SmOutput::ReplyCmd(Ok(())), SmOutput::RequestExit { .. }]
    ));
}
