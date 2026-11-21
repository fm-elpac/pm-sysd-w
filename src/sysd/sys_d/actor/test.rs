//! SysD actor 集成测试 (顶层路由与退出/重启).

use tokio::sync::oneshot;

use crate::{
    jsonl::{
        SdReq, SdReqLogQuery, SdReqLogQueryFilter, SdReqLogRO, SdReqPagePos, SdReqSysd,
        SdReqUnitList, SdReqUnitOpRO, SdReqUnitdRO, SdReqWhoami, SdRes, SdResWhoami,
    },
    sysd::{SysDM, init::Shutdown, test_util::*},
    sysu::{EnableSta, LoadSta, UnitSta},
};

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
    assert!(matches!(exit_rx.await, Ok(Ok(Shutdown::Exit))));
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
    assert!(matches!(exit_rx.await, Ok(Ok(Shutdown::SoftReboot))));
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

    match handle(
        &sysd,
        &SdReq::UnitdRO(SdReqUnitdRO::ListUnits(SdReqUnitList {
            limit: 1000,
            cursor: None,
        })),
    )
    .await
    {
        SdRes::ListUnits(page) => assert_eq!(page.list.len(), 2),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(&sysd, &SdReq::UnitdRO(SdReqUnitdRO::ListTimers)).await {
        SdRes::Timers(page) => assert!(page.list.is_empty()),
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
        SdRes::Dependencies(page) => assert!(page.list.is_empty()),
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
async fn whoami_self_info() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    let resp = handle(&sysd, &SdReq::Whoami(SdReqWhoami { pid: None })).await;
    match resp {
        SdRes::Whoami(SdResWhoami {
            service: None,
            init: Some(init),
        }) => {
            assert_eq!(init.instance_type, "machine");
            assert_eq!(init.pid, std::process::id());
            assert_eq!(init.service_name, None);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[tokio::test]
async fn whoami_pid_matches_running_service() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();

    let st = status(&sysd, "t.service").await.unwrap();
    let pid = service_status(&st).pid.expect("running service has pid");

    let resp = handle(&sysd, &SdReq::Whoami(SdReqWhoami { pid: Some(pid) })).await;
    match resp {
        SdRes::Whoami(SdResWhoami {
            service: Some(name),
            init: None,
        }) => assert_eq!(name, "t.service"),
        other => panic!("unexpected response: {:?}", other),
    }
}

#[tokio::test]
async fn whoami_pid_not_found() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    let resp = handle(
        &sysd,
        &SdReq::Whoami(SdReqWhoami {
            pid: Some(99_999_999),
        }),
    )
    .await;
    match resp {
        SdRes::Err(msg) => assert!(msg.contains("does not belong")),
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
        .page_entries(None, None, true, SdReqPagePos::Start(0), 200)
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
            filter: SdReqLogQueryFilter {
                level: Some("info".to_string()),
                unit: Some("t.service".to_string()),
            },
            pos: SdReqPagePos::End(0),
            asc: false,
            limit: 10,
        })),
    )
    .await;
    match resp {
        SdRes::Logs(page) => assert!(page.list.iter().any(|e| e.msg == "hello")),
        other => panic!("unexpected response: {:?}", other),
    }
}
