//! sys_d 模块测试 (SysD 顶层路由与退出/重启, 状态机/actor 分支覆盖).
//!
//! 未覆盖的分支说明:
//! - `Sm::ShuttingDown` 与 `running` / `exec_actions_*` 中的 `unreachable!` 分支,
//!   按设计不可达, 不测试.
//! - actor 的 send 失败防御分支 (Logd stopped / UnitD stopped / `SysD::send` 发送到
//!   已退出 actor / `spawn_exit_wait` 内部错误) 需要注入已关闭的 channel 或让依赖
//!   组件中途停止, 通过公开 `SysD` 句柄难以确定性触发, 故不测试.
//! - `exec_actions_d` 处理 `SmOutput::ReplyResult` 的分支需要 actor 处于
//!   `StoppingUnits` 状态, 同样受退出竞态影响, 不单独测试; 该状态机决策本身由
//!   `sm_stopping_*` 测试覆盖.

use tokio::sync::oneshot;

use super::sm::{Sm, SmInput, SmOutput};

use crate::{
    jsonl::{
        SdReq, SdReqLogQuery, SdReqLogRO, SdReqSysd, SdReqUnit, SdReqUnitOp, SdReqUnitOpRO,
        SdReqUnitRO, SdReqUnitd, SdReqUnitdRO, SdRes,
    },
    sysd::{EnableSta, LoadSta, SysDM, SysdError, UnitSta, init::Shutdown, test_util::*},
};

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

#[tokio::test]
async fn exit_notifies_shutdown() {
    let text = multi_units(&[
        ("a.service", "default.target"),
        ("b.service", "default.target"),
    ]);
    let (sysd, _logd, _dir, exit_rx) = make_sysd_with_exit(&text);

    svc_start(&sysd, "a.service").await.unwrap();
    svc_start(&sysd, "b.service").await.unwrap();
    let resp = handle(&sysd, &SdReq::SysD(SdReqSysd::Exit)).await;
    assert!(matches!(resp, SdRes::Ok(_)));
    assert!(matches!(exit_rx.await, Ok(Shutdown::Exit)));
}

#[tokio::test]
async fn soft_reboot_notifies_shutdown() {
    let text = multi_units(&[
        ("a.service", "default.target"),
        ("b.service", "default.target"),
    ]);
    let (sysd, _logd, _dir, exit_rx) = make_sysd_with_exit(&text);

    svc_start(&sysd, "a.service").await.unwrap();
    svc_start(&sysd, "b.service").await.unwrap();
    let resp = handle(&sysd, &SdReq::SysD(SdReqSysd::SoftReboot)).await;
    assert!(matches!(resp, SdRes::Ok(_)));
    assert!(matches!(exit_rx.await, Ok(Shutdown::SoftReboot)));
}

#[tokio::test]
async fn sqlite_vacuum_ok() {
    let text = unit_conf(&ok_cmd(), "");
    let (sysd, _logd, _dir) = make_sysd(&text);

    let resp = handle(&sysd, &SdReq::SysD(SdReqSysd::SqliteVacuum)).await;
    assert!(matches!(resp, SdRes::Ok(_)));
}

#[tokio::test]
async fn handle_dispatch() {
    let text = unit_conf(&ok_cmd(), r#""env:Install.WantedBy" = "default.target""#);
    let (sysd, _logd, _dir) = make_sysd(&text);
    unit_enable(&sysd, "t.service", false).await.unwrap();

    match handle(&sysd, &SdReq::UnitdRO(SdReqUnitdRO::ListUnits)).await {
        SdRes::ListUnits(units) => assert_eq!(units.len(), 2),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(&sysd, &SdReq::UnitdRO(SdReqUnitdRO::ListTimers)).await {
        SdRes::Timers(timers) => assert!(timers.is_empty()),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(
        &sysd,
        &SdReq::UnitRO(unit_ro("t.service", SdReqUnitOpRO::UnitState)),
    )
    .await
    {
        SdRes::UnitState(state) => {
            assert_eq!(state.as_ref().unwrap().active, UnitSta::Inactive)
        }
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(
        &sysd,
        &SdReq::UnitRO(unit_ro("t.service", SdReqUnitOpRO::Status)),
    )
    .await
    {
        SdRes::Status(status) => assert_eq!(status.name, "t.service"),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(
        &sysd,
        &SdReq::UnitRO(unit_ro("t.service", SdReqUnitOpRO::Config)),
    )
    .await
    {
        SdRes::Config(config) => assert_eq!(config.name, "t.service"),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(
        &sysd,
        &SdReq::UnitRO(unit_ro("t.service", SdReqUnitOpRO::ListDependencies)),
    )
    .await
    {
        SdRes::Dependencies(deps) => assert!(deps.is_empty()),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(&sysd, &SdReq::UnitdRO(SdReqUnitdRO::GetDefault)).await {
        SdRes::DefaultTarget(target) => assert_eq!(target, "default.target"),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(
        &sysd,
        &SdReq::UnitRO(unit_ro("t.service", SdReqUnitOpRO::UnitState)),
    )
    .await
    {
        SdRes::UnitState(state) => {
            let s = state.unwrap();
            assert_eq!(s.enable, EnableSta::Enabled);
            assert_eq!(s.load, LoadSta::Loaded);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[tokio::test]
async fn self_actions_log_without_unit() {
    let (sysd, logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    daemon_reload(&sysd).await.unwrap();
    handle(&sysd, &SdReq::SysD(SdReqSysd::Exit)).await;
    logd.flush().await.unwrap();

    let entries = logd
        .page_entries(None, None, 200, None)
        .await
        .unwrap()
        .entries;
    assert!(
        entries
            .iter()
            .any(|e| e.unit.is_none() && e.msg == "daemon-reload")
    );
    assert!(
        entries
            .iter()
            .any(|e| e.unit.is_none() && e.msg == "exit requested")
    );
}

#[tokio::test]
async fn start_default_target() {
    let text = multi_units(&[
        ("a.service", "default.target"),
        ("b.service", "timers.target"),
        ("c.service", "default.target"),
        ("d.service", "default.target"),
    ]);
    let (sysd, _logd, _dir) = make_sysd(&text);

    unit_enable(&sysd, "a.service", false).await.unwrap();
    assert!(unit_enable(&sysd, "b.service", false).await.is_err());
    unit_enable(&sysd, "d.service", false).await.unwrap();
    unit_mask(&sysd, "d.service", false).await.unwrap();

    let (tx, rx) = oneshot::channel();
    sysd.send(SysDM::StartDefaultTarget(tx)).await.unwrap();
    rx.await.unwrap().unwrap();

    assert!(is_active(&sysd, "a.service").await);
    assert!(!is_active(&sysd, "b.service").await);
    assert!(!is_active(&sysd, "c.service").await);
    assert!(!is_active(&sysd, "d.service").await);
}

#[tokio::test]
async fn log_ro_forwarded() {
    let (sysd, logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    logd.write_with("info", Some("t.service"), "hello").unwrap();
    logd.flush().await.unwrap();

    let resp = handle(
        &sysd,
        &SdReq::LogRO(SdReqLogRO::Query(SdReqLogQuery {
            level: Some("info".to_string()),
            unit: Some("t.service".to_string()),
            limit: 10,
            cursor: None,
        })),
    )
    .await;
    match resp {
        SdRes::Logs { logs, .. } => assert!(logs.iter().any(|e| e.msg == "hello")),
        other => panic!("unexpected response: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// 状态机 (sm) 分支测试
// ---------------------------------------------------------------------------

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
            level: None,
            unit: None,
            limit: 10,
            cursor: None,
        },
    ))));
    assert!(matches!(s, Sm::Running));
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], SmOutput::SendLog(_)));
}

#[test]
fn sm_running_forwards_unit_requests() {
    let reqs = [
        SdReq::UnitdRO(SdReqUnitdRO::ListUnits),
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
    assert_eq!(out.len(), 2);
    assert!(matches!(&out[0], SmOutput::Reply(SdRes::Ok(None))));
    assert!(matches!(
        &out[1],
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
            level: None,
            unit: None,
            limit: 10,
            cursor: None,
        },
    ))));
    assert!(matches!(s, Sm::StoppingUnits { .. }));
    assert_eq!(out.len(), 1);
    assert!(matches!(&out[0], SmOutput::SendLog(_)));
}

#[test]
fn sm_stopping_forwards_readonly_unit_requests() {
    let reqs = [
        SdReq::UnitdRO(SdReqUnitdRO::ListUnits),
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
