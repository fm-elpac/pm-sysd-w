//! ServiceS actor 集成测试 (单个 service 运行时的集成测试).

use crate::{
    sysd::test_util::*,
    sysu::{EnableSta, LoadSta, ServiceSta, UnitSta, UnitSubSta},
};

#[tokio::test]
async fn start_stop_is_active_status() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    assert!(!is_active(&sysd, "t.service").await);

    svc_start(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);

    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Active);
    assert_eq!(st.sub, UnitSubSta::Service(ServiceSta::Running));
    assert!(service_status(&st).pid.is_some());
    assert_eq!(st.enable, EnableSta::Static);
    assert_eq!(st.load, LoadSta::Loaded);

    svc_stop(&sysd, "t.service").await.unwrap();
    assert!(!is_active(&sysd, "t.service").await);

    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Inactive);
    assert_eq!(st.sub, UnitSubSta::Service(ServiceSta::Dead));
    assert!(service_status(&st).pid.is_none());
}

#[tokio::test]
async fn start_idempotent() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();
    svc_start(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn reload_and_reload_missing() {
    let text = unit_conf(&run_cmd(), &reload_extra());
    let (sysd, _logd, _dir) = make_sysd(&text);
    svc_start(&sysd, "t.service").await.unwrap();
    svc_reload(&sysd, "t.service").await.unwrap();

    let (sysd2, _logd2, _dir2) = make_sysd(&unit_conf(&run_cmd(), ""));
    assert!(svc_reload(&sysd2, "t.service").await.is_err());
}

#[tokio::test]
async fn restart_active() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();
    svc_restart(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn try_restart() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    svc_try_restart(&sysd, "t.service").await.unwrap();
    assert!(!is_active(&sysd, "t.service").await);

    svc_start(&sysd, "t.service").await.unwrap();
    svc_try_restart(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn reload_or_restart() {
    let text = unit_conf(&ok_cmd(), &reload_extra());
    let (sysd, _logd, _dir) = make_sysd(&text);
    svc_reload_or_restart(&sysd, "t.service").await.unwrap();

    let (sysd2, _logd2, _dir2) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd2, "t.service").await.unwrap();
    svc_reload_or_restart(&sysd2, "t.service").await.unwrap();
    assert!(is_active(&sysd2, "t.service").await);
}

#[tokio::test]
async fn try_reload_or_restart() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_try_reload_or_restart(&sysd, "t.service").await.unwrap();
    assert!(!is_active(&sysd, "t.service").await);

    svc_start(&sysd, "t.service").await.unwrap();
    svc_try_reload_or_restart(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn kill() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();
    svc_kill(&sysd, "t.service").await.unwrap();
    assert!(!is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn oneshot_exits_inactive() {
    let text = unit_conf(&slow_ok_cmd(), r#""env:Service.Type" = "oneshot""#);
    let (sysd, _logd, _dir) = make_sysd(&text);

    // Type=oneshot: start 应等待 ExecStart 执行完成后再返回.
    svc_start(&sysd, "t.service").await.unwrap();

    assert!(!is_active(&sysd, "t.service").await);
    assert!(!is_failed(&sysd, "t.service").await);
    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(service_status(&st).exit_status, Some(0));
}

#[tokio::test]
async fn remain_after_exit() {
    let text = unit_conf(&ok_cmd(), r#""env:Service.RemainAfterExit" = "yes""#);
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move {
            let st = status(s, "t.service").await.unwrap();
            service_status(&st).pid.is_none()
        })
    })
    .await;

    assert!(is_active(&sysd, "t.service").await);
    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Active);
    assert_eq!(st.sub, UnitSubSta::Service(ServiceSta::Exited));
    assert_eq!(service_status(&st).exit_status, Some(0));
}

#[tokio::test]
async fn win_create_no_window_starts() {
    let text = unit_conf(&run_cmd(), r#""env:Win.CreateNoWindow" = "yes""#);
    let (sysd, _logd, _dir) = make_sysd(&text);
    svc_start(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn restart_always() {
    let text = unit_conf(
        &fail_cmd(),
        r#""env:Service.Restart" = "always"
"env:Service.RestartSec" = "0"
"env:Unit.StartLimitIntervalSec" = "0"
"#,
    );
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move {
            let st = status(s, "t.service").await.unwrap();
            service_status(&st).restart_count >= 1
        })
    })
    .await;

    assert!(!is_failed(&sysd, "t.service").await);
}

#[tokio::test]
async fn restart_no_marks_failed() {
    let text = unit_conf(&fail_cmd(), "");
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move {
            let st = status(s, "t.service").await.unwrap();
            st.active == UnitSta::Failed
        })
    })
    .await;

    assert!(is_failed(&sysd, "t.service").await);
    assert!(!is_active(&sysd, "t.service").await);
    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(service_status(&st).exit_status, Some(1));
    assert_eq!(service_status(&st).restart_count, 0);
}

#[tokio::test]
async fn restart_on_failure_success_no_restart() {
    let text = unit_conf(&ok_cmd(), r#""env:Service.Restart" = "on-failure""#);
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move {
            let st = status(s, "t.service").await.unwrap();
            service_status(&st).pid.is_none()
        })
    })
    .await;

    assert!(!is_failed(&sysd, "t.service").await);
    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(service_status(&st).restart_count, 0);
}

#[tokio::test]
async fn restart_on_failure_restarts() {
    let text = unit_conf(
        &fail_cmd(),
        r#""env:Service.Restart" = "on-failure"
"env:Service.RestartSec" = "0"
"env:Unit.StartLimitIntervalSec" = "0"
"#,
    );
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move {
            let st = status(s, "t.service").await.unwrap();
            service_status(&st).restart_count >= 1
        })
    })
    .await;

    assert!(!is_failed(&sysd, "t.service").await);
}

#[tokio::test]
async fn unit_state_states() {
    let text = unit_conf(&run_cmd(), "");
    let (sysd, _logd, _dir) = make_sysd(&text);

    let st = unit_state(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Inactive);
    assert_eq!(st.sub, UnitSubSta::Service(ServiceSta::Dead));
    assert!(unit_state(&sysd, "nope.service").await.is_none());

    svc_start(&sysd, "t.service").await.unwrap();
    let st = unit_state(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Active);

    svc_stop(&sysd, "t.service").await.unwrap();
    let st = unit_state(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Inactive);
}

#[tokio::test]
async fn captures_stdout_stderr() {
    let (sysd, logd, _dir) = make_sysd(&unit_conf(&out_err_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();
    wait_stdout_stderr(&logd, "t.service", "hello", "oops").await;
}

#[tokio::test]
async fn exec_reload_captures_output() {
    let text = format!(
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "t.service"
"env:Unit.Description" = "test service"
"env:Service.ExecStart" = "{}"
"env:Service.ExecReload" = "{}"
"#,
        toml_str(&run_cmd()),
        toml_str(&out_err_cmd())
    );
    let (sysd, logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    svc_reload(&sysd, "t.service").await.unwrap();
    wait_stdout_stderr(&logd, "t.service", "hello", "oops").await;
}

#[tokio::test]
async fn oneshot_timeout_start_fails() {
    let text = unit_conf(
        &run_cmd(),
        r#""env:Service.Type" = "oneshot"
"env:Service.TimeoutStartSec" = "1"
"#,
    );
    let (sysd, _logd, _dir) = make_sysd(&text);

    let begin = std::time::Instant::now();
    assert!(svc_start(&sysd, "t.service").await.is_err());
    assert!(begin.elapsed() < std::time::Duration::from_secs(5));

    assert!(is_failed(&sysd, "t.service").await);
}

#[tokio::test]
async fn start_limit_hit_manual() {
    let text = unit_conf(
        &fail_cmd(),
        r#""env:Service.Type" = "oneshot"
"env:Unit.StartLimitBurst" = "2"
"env:Unit.StartLimitIntervalSec" = "60"
"#,
    );
    let (sysd, _logd, _dir) = make_sysd(&text);

    assert!(svc_start(&sysd, "t.service").await.is_err());
    assert!(svc_start(&sysd, "t.service").await.is_err());

    let e = svc_start(&sysd, "t.service").await.unwrap_err();
    assert!(e.to_string().contains("start-limit-hit"), "{}", e);
}

#[tokio::test]
async fn start_limit_hit_auto_restart() {
    let text = unit_conf(
        &fail_cmd(),
        r#""env:Service.Restart" = "always"
"env:Service.RestartSec" = "0"
"env:Unit.StartLimitBurst" = "3"
"env:Unit.StartLimitIntervalSec" = "60"
"#,
    );
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move { is_failed(s, "t.service").await })
    })
    .await;

    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(service_status(&st).restart_count, 2);
}

#[tokio::test]
async fn exec_stop_captures_output() {
    let text = format!(
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "t.service"
"env:Unit.Description" = "test service"
"env:Service.ExecStart" = "{}"
"env:Service.ExecStop" = "{}"
"#,
        toml_str(&run_cmd()),
        toml_str(&out_err_cmd())
    );
    let (sysd, logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    svc_stop(&sysd, "t.service").await.unwrap();
    wait_stdout_stderr(&logd, "t.service", "hello", "oops").await;
}
