//! sysd 模块测试.

use std::sync::Arc;

use super::*;
use crate::{
    Platform, conf,
    db::Db,
    jsonl::{ActiveState, Command, EnableState, Response, SubState},
    testutil::TempDir,
};

/// 一个 立即成功退出 (退出码 0) 的 命令 (跨平台).
fn ok_cmd() -> String {
    if cfg!(target_os = "windows") {
        "cmd /c exit 0".to_string()
    } else {
        "true".to_string()
    }
}

/// 一个 立即失败退出 (退出码 1) 的 命令 (跨平台).
fn fail_cmd() -> String {
    if cfg!(target_os = "windows") {
        "cmd /c exit 1".to_string()
    } else {
        "false".to_string()
    }
}

/// 一条 同时向 stdout / stderr 各输出一行 的 命令 (跨平台).
fn out_err_cmd() -> String {
    if cfg!(target_os = "windows") {
        "cmd /c \"echo hello & echo oops 1>&2\"".to_string()
    } else {
        "sh -c \"echo hello; echo oops 1>&2\"".to_string()
    }
}

/// 带 `ExecReload` 的 额外配置 (跨平台).
fn reload_extra() -> String {
    format!(r#""env:Service.ExecReload" = "{}""#, ok_cmd())
}

/// 把 字符串 转义为 TOML 基本字符串 (转义 `\` 与 `"`).
fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}

/// 单个 unit 的 配置文本 (`ExecStart` 可指定, `extra` 追加其它 `env:` 字段).
fn unit_conf(exec_start: &str, extra: &str) -> String {
    format!(
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "t.service"
"env:Unit.Description" = "test service"
"env:Service.ExecStart" = "{}"
{}"#,
        toml_str(exec_start),
        extra
    )
}

/// 多个 unit 的 配置文本 (每个 unit 指定 名称 与 `Install.WantedBy`).
fn multi_units(units: &[(&str, &str)]) -> String {
    let mut s = String::from("pm_tc = 1\n\n");
    for (name, wanted_by) in units {
        s.push_str(&format!(
            "[[t.sysd]]\n\"env:Unit\" = \"{}\"\n\"env:Service.ExecStart\" = \"{}\"\n\"env:Install.WantedBy\" = \"{}\"\n\n",
            name,
            ok_cmd(),
            wanted_by
        ));
    }
    s
}

/// 写入 配置 并 构造 [`Sysd`], 返回 (sysd, db, 临时目录).
fn make_sysd(text: &str) -> (Sysd<Platform>, Arc<Db>, TempDir) {
    let dir = TempDir::new("sysd");
    let conf_path = dir.path().join("c.toml");
    std::fs::write(&conf_path, text).unwrap();
    let conf_path = conf_path.to_string_lossy().into_owned();

    let conf = conf::load_conf(&conf_path).unwrap();
    let db = Arc::new(Db::open(&conf.db_path).unwrap());
    let pa = Arc::new(Platform::default());
    let sysd = Sysd::new(pa, db.clone(), &conf_path, conf.units);
    (sysd, db, dir)
}

/// 反复执行 [`monitor_tick`] 直到 谓词 满足 (超时 则 panic).
async fn pump_until(sysd: &Sysd<Platform>, pred: impl Fn(&State) -> bool) {
    let state = sysd.state.clone();
    let logd = sysd.logd.clone();
    let pa = sysd.pa.clone();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);

    loop {
        monitor_tick(&state, &logd, &pa).await;
        {
            let s = state.lock().await;
            if pred(&s) {
                return;
            }
        }
        if std::time::Instant::now() > deadline {
            panic!("monitor condition not reached");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// 反复读取 unit 日志, 直到 同时出现 指定 stdout / stderr 记录 (超时 则 panic).
async fn wait_stdout_stderr(sysd: &Sysd<Platform>, unit: &str, out: &str, err: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let entries = sysd.logd.recent_entries(Some(unit), 200).unwrap();
        let has_out = entries
            .iter()
            .any(|e| e.level == "stdout" && e.msg.contains(out));
        let has_err = entries
            .iter()
            .any(|e| e.level == "stderr" && e.msg.contains(err));
        if has_out && has_err {
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!("stdout/stderr not captured: {:?}", entries);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn start_stop_is_active_status() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));

    assert!(!sysd.is_active("t.service").await.unwrap());

    sysd.start("t.service").await.unwrap();
    assert!(sysd.is_active("t.service").await.unwrap());

    let st = sysd.status("t.service").await.unwrap();
    assert_eq!(st.active, ActiveState::Active);
    assert_eq!(st.sub, SubState::Running);
    assert!(st.pid.is_some());
    assert!(!st.enabled);
    assert!(!st.masked);

    sysd.stop("t.service").await.unwrap();
    assert!(!sysd.is_active("t.service").await.unwrap());

    let st = sysd.status("t.service").await.unwrap();
    assert_eq!(st.active, ActiveState::Inactive);
    assert_eq!(st.sub, SubState::Dead);
    assert!(st.pid.is_none());
}

#[tokio::test]
async fn start_idempotent() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));
    sysd.start("t.service").await.unwrap();
    sysd.start("t.service").await.unwrap();
    assert!(sysd.is_active("t.service").await.unwrap());
}

#[tokio::test]
async fn unknown_unit() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));
    assert!(sysd.start("nope.service").await.is_err());
    assert!(sysd.stop("nope.service").await.is_err());
    assert!(sysd.status("nope.service").await.is_err());
}

#[tokio::test]
async fn list_units_sorted() {
    let text = multi_units(&[
        ("b.service", "default.target"),
        ("a.service", "default.target"),
    ]);
    let (sysd, _db, _dir) = make_sysd(&text);

    let units = sysd.list_units().await.unwrap();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].name, "a.service");
    assert_eq!(units[1].name, "b.service");
}

#[tokio::test]
async fn unit_config() {
    let text = unit_conf(
        "bin --x",
        r#""env:Service.Restart" = "always"
"env:Service.RestartSec" = "3"
"env:Service.ExecStop" = "stop"
"env:Service.ExecReload" = "reload"
"env:Install.WantedBy" = "default.target"
"#,
    );
    let (sysd, _db, _dir) = make_sysd(&text);

    let c = sysd.unit_config("t.service").await.unwrap();
    assert_eq!(c.name, "t.service");
    assert_eq!(c.exec_start, "bin --x");
    assert_eq!(c.restart, "always");
    assert_eq!(c.restart_sec, 3);
    assert_eq!(c.exec_stop.as_deref(), Some("stop"));
    assert_eq!(c.exec_reload.as_deref(), Some("reload"));
    assert_eq!(c.wanted_by.as_deref(), Some("default.target"));
}

#[tokio::test]
async fn list_dependencies() {
    let text = unit_conf(&ok_cmd(), r#""env:Install.WantedBy" = "default.target""#);
    let (sysd, _db, _dir) = make_sysd(&text);
    assert_eq!(
        sysd.list_dependencies("t.service").await.unwrap(),
        vec!["default.target".to_string()]
    );

    let (sysd2, _db2, _dir2) = make_sysd(&unit_conf(&ok_cmd(), ""));
    assert!(
        sysd2
            .list_dependencies("t.service")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn reload_and_reload_missing() {
    let text = unit_conf(&ok_cmd(), &reload_extra());
    let (sysd, _db, _dir) = make_sysd(&text);
    sysd.start("t.service").await.unwrap();
    sysd.reload("t.service").await.unwrap();

    let (sysd2, _db2, _dir2) = make_sysd(&unit_conf(&ok_cmd(), ""));
    assert!(sysd2.reload("t.service").await.is_err());
}

#[tokio::test]
async fn restart_active() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));
    sysd.start("t.service").await.unwrap();
    sysd.restart("t.service").await.unwrap();
    assert!(sysd.is_active("t.service").await.unwrap());
}

#[tokio::test]
async fn try_restart() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));

    // 未 active: 不动作.
    sysd.try_restart("t.service").await.unwrap();
    assert!(!sysd.is_active("t.service").await.unwrap());

    // active: 重启.
    sysd.start("t.service").await.unwrap();
    sysd.try_restart("t.service").await.unwrap();
    assert!(sysd.is_active("t.service").await.unwrap());
}

#[tokio::test]
async fn reload_or_restart() {
    let text = unit_conf(&ok_cmd(), &reload_extra());
    let (sysd, _db, _dir) = make_sysd(&text);
    sysd.reload_or_restart("t.service").await.unwrap();

    let (sysd2, _db2, _dir2) = make_sysd(&unit_conf(&ok_cmd(), ""));
    sysd2.start("t.service").await.unwrap();
    sysd2.reload_or_restart("t.service").await.unwrap();
    assert!(sysd2.is_active("t.service").await.unwrap());
}

#[tokio::test]
async fn try_reload_or_restart() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));
    sysd.try_reload_or_restart("t.service").await.unwrap();
    assert!(!sysd.is_active("t.service").await.unwrap());

    sysd.start("t.service").await.unwrap();
    sysd.try_reload_or_restart("t.service").await.unwrap();
    assert!(sysd.is_active("t.service").await.unwrap());
}

#[tokio::test]
async fn kill() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));
    sysd.start("t.service").await.unwrap();
    sysd.kill("t.service").await.unwrap();
    assert!(!sysd.is_active("t.service").await.unwrap());
}

#[tokio::test]
async fn enable_disable_is_enabled() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));

    assert_eq!(
        sysd.enable_state("t.service").await.unwrap(),
        EnableState::Disabled
    );

    sysd.enable("t.service", false).await.unwrap();
    assert_eq!(
        sysd.enable_state("t.service").await.unwrap(),
        EnableState::Enabled
    );

    sysd.mask("t.service").await.unwrap();
    assert_eq!(
        sysd.enable_state("t.service").await.unwrap(),
        EnableState::Masked
    );

    sysd.unmask("t.service").await.unwrap();
    sysd.disable("t.service", false).await.unwrap();
    assert_eq!(
        sysd.enable_state("t.service").await.unwrap(),
        EnableState::Disabled
    );
}

#[tokio::test]
async fn enable_now_starts_and_disable_now_stops() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));

    sysd.enable("t.service", true).await.unwrap();
    assert!(sysd.is_active("t.service").await.unwrap());

    sysd.disable("t.service", true).await.unwrap();
    assert!(!sysd.is_active("t.service").await.unwrap());
    assert_eq!(
        sysd.enable_state("t.service").await.unwrap(),
        EnableState::Disabled
    );
}

#[tokio::test]
async fn reenable() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));
    sysd.enable("t.service", false).await.unwrap();
    sysd.disable("t.service", false).await.unwrap();
    sysd.reenable("t.service").await.unwrap();
    assert_eq!(
        sysd.enable_state("t.service").await.unwrap(),
        EnableState::Enabled
    );
}

#[tokio::test]
async fn mask_stops_and_blocks_start() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));
    sysd.start("t.service").await.unwrap();
    sysd.mask("t.service").await.unwrap();
    assert!(!sysd.is_active("t.service").await.unwrap());
    assert_eq!(
        sysd.enable_state("t.service").await.unwrap(),
        EnableState::Masked
    );
    assert!(sysd.start("t.service").await.is_err());

    sysd.unmask("t.service").await.unwrap();
    sysd.start("t.service").await.unwrap();
    assert!(sysd.is_active("t.service").await.unwrap());
}

#[tokio::test]
async fn get_default_set_default() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));
    assert_eq!(sysd.get_default().await.unwrap(), "default.target");

    sysd.set_default("foo.target").await.unwrap();
    assert_eq!(sysd.get_default().await.unwrap(), "foo.target");
}

#[tokio::test]
async fn daemon_reload_reloads_units() {
    let (sysd, _db, dir) = make_sysd(&unit_conf(&ok_cmd(), ""));
    assert_eq!(sysd.list_units().await.unwrap().len(), 1);

    let new_text = multi_units(&[
        ("a.service", "default.target"),
        ("b.service", "default.target"),
    ]);
    std::fs::write(dir.path().join("c.toml"), new_text).unwrap();
    sysd.daemon_reload().await.unwrap();

    let units = sysd.list_units().await.unwrap();
    assert_eq!(units.len(), 2);
    assert_eq!(units[0].name, "a.service");
}

#[tokio::test]
async fn start_default_target() {
    let text = multi_units(&[
        ("a.service", "default.target"),
        ("b.service", "timers.target"),
        ("c.service", "default.target"),
        ("d.service", "default.target"),
    ]);
    let (sysd, _db, _dir) = make_sysd(&text);

    sysd.enable("a.service", false).await.unwrap();
    sysd.enable("b.service", false).await.unwrap();
    sysd.enable("d.service", false).await.unwrap();
    sysd.mask("d.service").await.unwrap();

    sysd.start_default_target().await.unwrap();

    assert!(sysd.is_active("a.service").await.unwrap());
    assert!(!sysd.is_active("b.service").await.unwrap());
    assert!(!sysd.is_active("c.service").await.unwrap());
    assert!(!sysd.is_active("d.service").await.unwrap());
}

#[tokio::test]
async fn stop_all_soft_reboot_exit() {
    let text = multi_units(&[
        ("a.service", "default.target"),
        ("b.service", "default.target"),
    ]);
    let (sysd, _db, _dir) = make_sysd(&text);

    sysd.start("a.service").await.unwrap();
    sysd.start("b.service").await.unwrap();
    sysd.handle(&Command::SoftReboot).await;
    assert!(!sysd.is_active("a.service").await.unwrap());
    assert!(!sysd.is_active("b.service").await.unwrap());

    sysd.start("a.service").await.unwrap();
    sysd.handle(&Command::Exit).await;
    assert!(!sysd.is_active("a.service").await.unwrap());
}

#[tokio::test]
async fn handle_dispatch() {
    let text = unit_conf(&ok_cmd(), r#""env:Install.WantedBy" = "default.target""#);
    let (sysd, _db, _dir) = make_sysd(&text);
    sysd.enable("t.service", false).await.unwrap();

    match sysd.handle(&Command::ListUnits).await {
        Response::ListUnits { units } => assert_eq!(units.len(), 1),
        other => panic!("unexpected response: {:?}", other),
    }
    match sysd.handle(&Command::ListTimers).await {
        Response::Timers { timers } => assert!(timers.is_empty()),
        other => panic!("unexpected response: {:?}", other),
    }
    match sysd
        .handle(&Command::IsActive {
            unit: "t.service".into(),
        })
        .await
    {
        Response::Bool { value } => assert!(!value),
        other => panic!("unexpected response: {:?}", other),
    }
    match sysd
        .handle(&Command::Status {
            unit: "t.service".into(),
        })
        .await
    {
        Response::Status { status } => assert_eq!(status.name, "t.service"),
        other => panic!("unexpected response: {:?}", other),
    }
    match sysd
        .handle(&Command::Show {
            unit: "t.service".into(),
        })
        .await
    {
        Response::Config { config } => assert_eq!(config.name, "t.service"),
        other => panic!("unexpected response: {:?}", other),
    }
    match sysd
        .handle(&Command::Cat {
            unit: "t.service".into(),
        })
        .await
    {
        Response::Config { .. } => {}
        other => panic!("unexpected response: {:?}", other),
    }
    match sysd
        .handle(&Command::ListDependencies {
            unit: "t.service".into(),
        })
        .await
    {
        Response::Dependencies { deps } => {
            assert_eq!(deps, vec!["default.target".to_string()])
        }
        other => panic!("unexpected response: {:?}", other),
    }
    match sysd.handle(&Command::GetDefault).await {
        Response::DefaultTarget { target } => assert_eq!(target, "default.target"),
        other => panic!("unexpected response: {:?}", other),
    }
    match sysd
        .handle(&Command::IsEnabled {
            unit: "t.service".into(),
        })
        .await
    {
        Response::EnableState { state } => assert_eq!(state, EnableState::Enabled),
        other => panic!("unexpected response: {:?}", other),
    }
}

#[tokio::test]
async fn handle_todo_commands() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));

    let cmds = vec![
        Command::Clean {
            unit: "t.service".into(),
        },
        Command::Preset {
            unit: "t.service".into(),
        },
        Command::ShowEnvironment,
        Command::SetEnvironment,
        Command::UnsetEnvironment,
        Command::ImportEnvironment,
    ];
    for cmd in cmds {
        match sysd.handle(&cmd).await {
            Response::Err { message } => assert!(message.contains("TODO")),
            other => panic!("unexpected response: {:?}", other),
        }
    }
}

#[tokio::test]
async fn oneshot_exits_inactive() {
    let text = unit_conf(&ok_cmd(), r#""env:Service.Type" = "oneshot""#);
    let (sysd, _db, _dir) = make_sysd(&text);

    sysd.start("t.service").await.unwrap();
    assert!(sysd.is_active("t.service").await.unwrap());

    pump_until(&sysd, |s| {
        s.procs
            .get("t.service")
            .map(|p| p.child.is_none() && !p.active)
            .unwrap_or(false)
    })
    .await;

    assert!(!sysd.is_active("t.service").await.unwrap());
    assert!(!sysd.is_failed("t.service").await.unwrap());
    let st = sysd.status("t.service").await.unwrap();
    assert_eq!(st.exit_status, Some(0));
}

#[tokio::test]
async fn remain_after_exit() {
    let text = unit_conf(&ok_cmd(), r#""env:Service.RemainAfterExit" = "yes""#);
    let (sysd, _db, _dir) = make_sysd(&text);

    sysd.start("t.service").await.unwrap();
    pump_until(&sysd, |s| {
        s.procs
            .get("t.service")
            .map(|p| p.child.is_none())
            .unwrap_or(false)
    })
    .await;

    assert!(sysd.is_active("t.service").await.unwrap());
    let st = sysd.status("t.service").await.unwrap();
    assert_eq!(st.active, ActiveState::Active);
    assert_eq!(st.sub, SubState::Exited);
    assert_eq!(st.exit_status, Some(0));
}

#[tokio::test]
async fn win_create_no_window_starts() {
    let text = unit_conf(&ok_cmd(), r#""env:Win.CreateNoWindow" = "yes""#);
    let (sysd, _db, _dir) = make_sysd(&text);
    sysd.start("t.service").await.unwrap();
    assert!(sysd.is_active("t.service").await.unwrap());
}

#[tokio::test]
async fn restart_always() {
    let text = unit_conf(
        &fail_cmd(),
        r#""env:Service.Restart" = "always"
"env:Service.RestartSec" = "0"
"#,
    );
    let (sysd, _db, _dir) = make_sysd(&text);

    sysd.start("t.service").await.unwrap();
    pump_until(&sysd, |s| {
        s.procs
            .get("t.service")
            .map(|p| p.restart_count >= 1)
            .unwrap_or(false)
    })
    .await;

    let s = sysd.state.lock().await;
    let p = s.procs.get("t.service").unwrap();
    assert!(p.child.is_some());
    assert!(p.active);
    assert!(!p.failed);
    drop(s);

    assert!(!sysd.is_failed("t.service").await.unwrap());
}

#[tokio::test]
async fn restart_no_marks_failed() {
    let text = unit_conf(&fail_cmd(), "");
    let (sysd, _db, _dir) = make_sysd(&text);

    sysd.start("t.service").await.unwrap();
    pump_until(&sysd, |s| {
        s.procs
            .get("t.service")
            .map(|p| p.child.is_none() && p.failed)
            .unwrap_or(false)
    })
    .await;

    assert!(sysd.is_failed("t.service").await.unwrap());
    assert!(!sysd.is_active("t.service").await.unwrap());
    let st = sysd.status("t.service").await.unwrap();
    assert_eq!(st.exit_status, Some(1));
    assert_eq!(st.restart_count, 0);
}

#[tokio::test]
async fn restart_on_failure_success_no_restart() {
    let text = unit_conf(&ok_cmd(), r#""env:Service.Restart" = "on-failure""#);
    let (sysd, _db, _dir) = make_sysd(&text);

    sysd.start("t.service").await.unwrap();
    pump_until(&sysd, |s| {
        s.procs
            .get("t.service")
            .map(|p| p.child.is_none())
            .unwrap_or(false)
    })
    .await;

    assert!(!sysd.is_failed("t.service").await.unwrap());
    assert_eq!(sysd.status("t.service").await.unwrap().restart_count, 0);
}

#[tokio::test]
async fn restart_on_failure_restarts() {
    let text = unit_conf(
        &fail_cmd(),
        r#""env:Service.Restart" = "on-failure"
"env:Service.RestartSec" = "0"
"#,
    );
    let (sysd, _db, _dir) = make_sysd(&text);

    sysd.start("t.service").await.unwrap();
    pump_until(&sysd, |s| {
        s.procs
            .get("t.service")
            .map(|p| p.restart_count >= 1)
            .unwrap_or(false)
    })
    .await;

    assert!(!sysd.is_failed("t.service").await.unwrap());
}

#[tokio::test]
async fn reset_failed() {
    let text = unit_conf(&fail_cmd(), "");
    let (sysd, _db, _dir) = make_sysd(&text);

    // 失败 后 指定 unit 清除.
    sysd.start("t.service").await.unwrap();
    pump_until(&sysd, |s| {
        s.procs.get("t.service").map(|p| p.failed).unwrap_or(false)
    })
    .await;
    assert!(sysd.is_failed("t.service").await.unwrap());
    sysd.reset_failed(Some("t.service")).await.unwrap();
    assert!(!sysd.is_failed("t.service").await.unwrap());

    // 再次失败 后 清除全部.
    sysd.start("t.service").await.unwrap();
    pump_until(&sysd, |s| {
        s.procs.get("t.service").map(|p| p.failed).unwrap_or(false)
    })
    .await;
    assert!(sysd.is_failed("t.service").await.unwrap());
    sysd.reset_failed(None).await.unwrap();
    assert!(!sysd.is_failed("t.service").await.unwrap());
}

#[tokio::test]
async fn captures_stdout_stderr() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&out_err_cmd(), ""));
    sysd.start("t.service").await.unwrap();
    wait_stdout_stderr(&sysd, "t.service", "hello", "oops").await;
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
        toml_str(&ok_cmd()),
        toml_str(&out_err_cmd())
    );
    let (sysd, _db, _dir) = make_sysd(&text);

    sysd.reload("t.service").await.unwrap();
    wait_stdout_stderr(&sysd, "t.service", "hello", "oops").await;
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
        toml_str(&ok_cmd()),
        toml_str(&out_err_cmd())
    );
    let (sysd, _db, _dir) = make_sysd(&text);

    sysd.start("t.service").await.unwrap();
    sysd.stop("t.service").await.unwrap();
    wait_stdout_stderr(&sysd, "t.service", "hello", "oops").await;
}

#[tokio::test]
async fn self_actions_log_without_unit() {
    let (sysd, _db, _dir) = make_sysd(&unit_conf(&ok_cmd(), ""));

    sysd.daemon_reload().await.unwrap();
    sysd.handle(&Command::Exit).await;
    sysd.flush_logs().unwrap();

    let entries = sysd.logd.recent_entries(None, 200).unwrap();
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
