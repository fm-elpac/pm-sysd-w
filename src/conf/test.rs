//! conf 模块测试.

use crate::test::TempDir;

use super::{RestartPolicy, ServiceType, UnitType, load_conf};

/// 写入 配置文本 到 临时目录, 返回 配置文件 路径.
fn write_conf(dir: &TempDir, text: &str) -> String {
    let path = dir.path().join("c.toml");
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

/// 最小 合法配置.
fn basic_conf() -> &'static str {
    r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"#
}

/// 带 `[env] SYSD_DB` 的 配置.
fn conf_with_db(db: &str) -> String {
    format!(
        r#"pm_tc = 1

[env]
SYSD_DB = "{}"

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"#,
        db
    )
}

#[test]
fn load_units() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"
pm_tc = 1

[[t.sysd]]
"env:Unit" = "test-run.service"
"env:Unit.Description" = "Run a test program"
"env:Service.ExecStart" = "path/to/bin --arg"
"env:Service.Restart" = "always"
"env:Service.RestartSec" = "10"
"env:Install.WantedBy" = "default.target"
"#,
    );

    let conf = load_conf(&path).unwrap();
    assert_eq!(conf.units.len(), 1);

    let u = &conf.units[0];
    assert_eq!(u.name, "test-run.service");
    assert_eq!(u.unit_type, UnitType::Service);
    assert_eq!(u.description, "Run a test program");
    assert_eq!(
        u.service.exec_start,
        vec![vec!["path/to/bin".to_string(), "--arg".to_string()]]
    );
    assert_eq!(u.service.restart, RestartPolicy::Always);
    assert_eq!(u.service.restart_sec, 10);
    assert_eq!(u.service.service_type, ServiceType::Simple);
    assert_eq!(u.install.wanted_by, vec!["default.target".to_string()]);
}

#[test]
fn defaults() {
    let dir = TempDir::new("conf");
    let path = write_conf(&dir, basic_conf());

    let conf = load_conf(&path).unwrap();
    let u = &conf.units[0];
    assert_eq!(u.description, "");
    assert_eq!(u.service.service_type, ServiceType::Simple);
    assert_eq!(u.service.restart, RestartPolicy::No);
    assert_eq!(u.service.restart_sec, 1);
    assert_eq!(u.start_limit_interval_sec, 10);
    assert_eq!(u.start_limit_burst, 5);
    assert_eq!(u.service.timeout_start_sec, 90);
    assert_eq!(u.service.timeout_stop_sec, 90);
    assert!(u.service.working_directory.is_none());
    assert!(u.service.environment.is_empty());
    assert!(!u.service.remain_after_exit);
    assert!(u.service.exec_stop.is_empty());
    assert!(u.service.exec_reload.is_empty());
    assert!(u.install.wanted_by.is_empty());
    assert!(!u.win.create_no_window);
    assert!(conf.db_path.ends_with("pm_sysd_w_v1.db"));
}

#[test]
fn start_limit_fields() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Unit.StartLimitIntervalSec" = "60"
"env:Unit.StartLimitBurst" = "10"
"#,
    );

    let conf = load_conf(&path).unwrap();
    // 用户 在配置中 指定时 读取并解析.
    assert_eq!(conf.units[0].start_limit_interval_sec, 60);
    assert_eq!(conf.units[0].start_limit_burst, 10);
    // 已支持 的字段 不产生 警告.
    assert!(
        !conf
            .warnings
            .iter()
            .any(|w| w.contains("StartLimitIntervalSec"))
    );
    assert!(!conf.warnings.iter().any(|w| w.contains("StartLimitBurst")));
}

#[test]
fn bad_start_limit() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Unit.StartLimitIntervalSec" = "abc"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("StartLimitIntervalSec"));
}

#[test]
fn empty_conf() {
    let dir = TempDir::new("conf");
    let path = write_conf(&dir, "pm_tc = 1\n");

    let conf = load_conf(&path).unwrap();
    assert!(conf.units.is_empty());
    assert!(conf.db_path.ends_with("pm_sysd_w_v1.db"));
}

#[test]
fn full_fields() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"
pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Unit.Description" = "d"
"env:Service.Type" = "oneshot"
"env:Service.ExecStart" = "bin"
"env:Service.ExecStop" = "stop"
"env:Service.ExecReload" = "reload"
"env:Service.Restart" = "on-failure"
"env:Service.RestartSec" = "5"
"env:Service.TimeoutStartSec" = "10"
"env:Service.TimeoutStopSec" = "20"
"env:Service.WorkingDirectory" = "/tmp/wd"
"env:Service.Environment" = "A=B"
"env:Service.RemainAfterExit" = "yes"
"env:Install.WantedBy" = "default.target"
"#,
    );

    let conf = load_conf(&path).unwrap();
    let u = &conf.units[0];
    assert_eq!(u.service.service_type, ServiceType::Oneshot);
    assert_eq!(u.service.exec_stop, vec![vec!["stop".to_string()]]);
    assert_eq!(u.service.exec_reload, vec![vec!["reload".to_string()]]);
    assert_eq!(u.service.restart, RestartPolicy::OnFailure);
    assert_eq!(u.service.restart_sec, 5);
    assert_eq!(u.service.timeout_start_sec, 10);
    assert_eq!(u.service.timeout_stop_sec, 20);
    assert_eq!(u.service.working_directory.as_deref(), Some("/tmp/wd"));
    assert_eq!(
        u.service.environment,
        vec![("A".to_string(), "B".to_string())]
    );
    assert!(u.service.remain_after_exit);
    assert_eq!(u.install.wanted_by, vec!["default.target".to_string()]);
}

#[test]
fn db_path_resolution() {
    let dir = TempDir::new("conf");

    // 默认: 相对主配置目录 的 pm_sysd_w_v1.db.
    let default = write_conf(&dir, basic_conf());
    assert!(
        load_conf(&default)
            .unwrap()
            .db_path
            .ends_with("pm_sysd_w_v1.db")
    );

    // 相对路径: 相对主配置目录.
    let rel = write_conf(&dir, &conf_with_db("sub/x.db"));
    let rp = load_conf(&rel).unwrap().db_path;
    assert!(rp.ends_with("x.db"));
    assert!(rp.contains("sub"));

    // 绝对路径: 原样保留.
    #[cfg(target_os = "windows")]
    let abs = "C:/pm_sysd_w_test_abs.db";
    #[cfg(not(target_os = "windows"))]
    let abs = "/pm_sysd_w_test_abs.db";
    let ab = write_conf(&dir, &conf_with_db(abs));
    assert_eq!(load_conf(&ab).unwrap().db_path, abs);
}

#[test]
fn multiple_units() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin-a"

[[t.sysd]]
"env:Unit" = "b.service"
"env:Service.ExecStart" = "bin-b"
"#,
    );

    let conf = load_conf(&path).unwrap();
    assert_eq!(conf.units.len(), 2);
    assert_eq!(conf.units[0].name, "a.service");
    assert_eq!(
        conf.units[0].service.exec_start,
        vec![vec!["bin-a".to_string()]]
    );
    assert_eq!(conf.units[1].name, "b.service");
    assert_eq!(
        conf.units[1].service.exec_start,
        vec![vec!["bin-b".to_string()]]
    );
}

#[test]
fn duplicate_unit_name() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin-a"

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin-b"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("duplicate unit"));
}

#[test]
fn missing_unit_name() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Service.ExecStart" = "bin"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("Unit"));
}

#[test]
fn missing_exec_start() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "bad.service"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("ExecStart"));
}

#[test]
fn exec_start_and_stop_both_empty() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "bad.service"
"env:Service.ExecStart" = ""
"env:Service.ExecStop" = ""
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("must not both be empty"));
}

#[test]
fn simple_requires_exec_start() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "bad.service"
"env:Service.Type" = "simple"
"env:Service.ExecStop" = "stop"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("Type=simple requires ExecStart"));
}

#[test]
fn missing_exec_start_requires_remain_after_exit() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "bad.service"
"env:Service.Type" = "oneshot"
"env:Service.ExecStop" = "stop"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("RemainAfterExit=yes"));
}

#[test]
fn oneshot_exec_stop_only_with_remain_after_exit() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.Type" = "oneshot"
"env:Service.ExecStop" = "stop"
"env:Service.RemainAfterExit" = "yes"
"#,
    );
    let conf = load_conf(&path).unwrap();
    assert!(conf.units[0].service.exec_start.is_empty());
    assert_eq!(
        conf.units[0].service.exec_stop,
        vec![vec!["stop".to_string()]]
    );
    assert!(conf.units[0].service.remain_after_exit);
}

#[test]
fn multi_line_exec_commands() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin-a --x"
"env:Service.ExecStart.1" = "bin-b"
"env:Service.ExecStop.2" = "stop-b"
"env:Service.ExecStop.1" = "stop-a"
"#,
    );
    let conf = load_conf(&path).unwrap();
    let s = &conf.units[0].service;
    assert_eq!(
        s.exec_start,
        vec![
            vec!["bin-a".to_string(), "--x".to_string()],
            vec!["bin-b".to_string()],
        ]
    );
    assert_eq!(
        s.exec_stop,
        vec![vec!["stop-a".to_string()], vec!["stop-b".to_string()]]
    );
    // 多行 key 不再产生不支持警告
    assert!(
        !conf
            .warnings
            .iter()
            .any(|w| w.contains("ExecStart.1") || w.contains("ExecStop.1"))
    );
}

#[test]
fn multi_line_exec_empty_clears_previous() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1
[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin-a"
"env:Service.ExecStart.1" = ""
"env:Service.ExecStart.2" = "bin-b"
"#,
    );
    let conf = load_conf(&path).unwrap();
    assert_eq!(
        conf.units[0].service.exec_start,
        vec![vec!["bin-b".to_string()]]
    );
}

#[test]
fn multi_line_environment() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Service.Environment" = "A=B"
"env:Service.Environment.1" = "C=D"
"#,
    );
    let conf = load_conf(&path).unwrap();
    assert_eq!(
        conf.units[0].service.environment,
        vec![
            ("A".to_string(), "B".to_string()),
            ("C".to_string(), "D".to_string())
        ]
    );
}

#[test]
fn multi_line_environment_empty_clears_previous() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Service.Environment" = "A=B"
"env:Service.Environment.1" = ""
"env:Service.Environment.2" = "C=D"
"#,
    );
    let conf = load_conf(&path).unwrap();
    assert_eq!(
        conf.units[0].service.environment,
        vec![("C".to_string(), "D".to_string())]
    );
}

#[test]
fn multi_line_wanted_by_dedup() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Install.WantedBy" = "default.target"
"env:Install.WantedBy.1" = "x.target"
"env:Install.WantedBy.2" = "default.target"
"#,
    );
    let conf = load_conf(&path).unwrap();
    assert_eq!(
        conf.units[0].install.wanted_by,
        vec!["default.target".to_string(), "x.target".to_string()]
    );
    assert_eq!(conf.warnings.len(), 1);
    assert!(conf.warnings[0].contains("duplicate Install.WantedBy"));
    assert!(conf.warnings[0].contains("default.target"));
}

#[test]
fn multi_line_empty_suffix_error() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Service.ExecStart." = "bad"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("empty suffix"));
}

#[test]
fn unit_name_must_end_with_service() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "bad"
"env:Service.ExecStart" = "bin"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains(".service"));
}

#[test]
fn unit_name_check_name() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a b.service"
"env:Service.ExecStart" = "bin"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("invalid unit name"));
}

#[test]
fn unit_name_nfkc() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "①.service"
"env:Service.ExecStart" = "bin"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("invalid unit name"));
}

#[test]
fn unit_name_too_long() {
    let dir = TempDir::new("conf");
    let name = format!("{}.service", "a".repeat(500));
    let path = write_conf(
        &dir,
        &format!(
            r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "{}"
"env:Service.ExecStart" = "bin"
"#,
            name
        ),
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("exceeds 500 bytes"));
}

#[test]
fn unit_name_exact_limit_ok() {
    let dir = TempDir::new("conf");
    let name = format!("{}.service", "a".repeat(492));
    let path = write_conf(
        &dir,
        &format!(
            r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "{}"
"env:Service.ExecStart" = "bin"
"#,
            name
        ),
    );
    let conf = load_conf(&path).unwrap();
    assert_eq!(conf.units[0].name.len(), 500);
}

#[test]
fn unit_name_dot_service_empty() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = ".service"
"env:Service.ExecStart" = "bin"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("must not be empty"));
}

#[test]
fn timer_unit_not_implemented() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.timer"
"env:Timer.OnCalendar" = "daily"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("timer"));
}

#[test]
fn custom_target_not_supported() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "foo.target"
"env:Service.ExecStart" = "bin"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("target"));
}

#[test]
fn unknown_service_type() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.Type" = "forking"
"env:Service.ExecStart" = "bin"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("Service.Type"));
}

#[test]
fn unknown_restart() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Service.Restart" = "sometimes"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("Service.Restart"));
}

#[test]
fn bad_restart_sec() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Service.RestartSec" = "-1"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("RestartSec"));
}

#[test]
fn bad_timeout() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Service.TimeoutStartSec" = "abc"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("TimeoutStartSec"));
}

#[test]
fn bad_remain_after_exit() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Service.RemainAfterExit" = "maybe"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("RemainAfterExit"));
}

#[test]
fn win_create_no_window() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Win.CreateNoWindow" = "yes"
"#,
    );
    let conf = load_conf(&path).unwrap();
    assert!(conf.units[0].win.create_no_window);
}

#[test]
fn bad_win_create_no_window() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Win.CreateNoWindow" = "maybe"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("Win.CreateNoWindow"));
}

#[test]
fn bad_environment() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Service.Environment" = "NOEQUALS"
"#,
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("environment"));
}

#[test]
fn remain_after_exit_bool_keywords() {
    for (kw, want) in [
        ("yes", true),
        ("true", true),
        ("1", true),
        ("on", true),
        ("no", false),
        ("false", false),
        ("0", false),
        ("off", false),
    ] {
        let dir = TempDir::new("conf");
        let path = write_conf(
            &dir,
            &format!(
                r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Service.RemainAfterExit" = "{}"
"#,
                kw
            ),
        );
        assert_eq!(
            load_conf(&path).unwrap().units[0].service.remain_after_exit,
            want
        );
    }
}

#[test]
fn enum_as_str() {
    assert_eq!(ServiceType::Simple.as_str(), "simple");
    assert_eq!(ServiceType::Oneshot.as_str(), "oneshot");
    assert_eq!(RestartPolicy::No.as_str(), "no");
    assert_eq!(RestartPolicy::Always.as_str(), "always");
    assert_eq!(RestartPolicy::OnFailure.as_str(), "on-failure");
}

#[test]
fn unsupported_key_warnings() {
    let dir = TempDir::new("conf");
    let path = write_conf(
        &dir,
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "bin"
"env:Unit.After" = "x"
"env:Service.FooBar" = "x"
"env:Install.Alias" = "x"
"env:Timer.OnCalendar" = "x"
"env:Win.Other" = "x"
"env:Other.Key" = "x"
"#,
    );

    let conf = load_conf(&path).unwrap();
    assert_eq!(conf.units.len(), 1);

    let warnings = &conf.warnings;
    for key in [
        "Unit.After",
        "Service.FooBar",
        "Install.Alias",
        "Timer.OnCalendar",
        "Win.Other",
    ] {
        assert!(
            warnings.iter().any(|w| w.contains(key)),
            "missing warning for `{}`: {:?}",
            key,
            warnings
        );
    }
    // 其余前缀的 key 静默忽略, 不产生警告.
    assert!(!warnings.iter().any(|w| w.contains("Other.Key")));
}

#[test]
fn unit_value_too_large() {
    let dir = TempDir::new("conf");
    let value = "a".repeat(20 * 1024 + 1);
    let path = write_conf(
        &dir,
        &format!(
            r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "a.service"
"env:Service.ExecStart" = "{}"
"#,
            value
        ),
    );
    let e = load_conf(&path).unwrap_err();
    assert!(e.to_string().contains("exceeds 20480 bytes"));
}

#[test]
fn conf_total_size_limit() {
    use pm_tc::conf::ReadConfFile;

    let dir = TempDir::new("conf");
    let path = dir.path().join("a.toml");
    std::fs::write(&path, "12345").unwrap();
    let p = path.to_string_lossy().into_owned();

    let mut r = super::FsReader { total: 0, limit: 4 };
    let e = r.read(&p).unwrap_err();
    assert!(e.to_string().contains("total size"));
}

#[test]
fn conf_total_size_exact_ok() {
    use pm_tc::conf::ReadConfFile;

    let dir = TempDir::new("conf");
    let path = dir.path().join("a.toml");
    std::fs::write(&path, "1234").unwrap();
    let p = path.to_string_lossy().into_owned();

    let mut r = super::FsReader { total: 0, limit: 4 };
    let text = r.read(&p).unwrap();
    assert_eq!(text, "1234");
    assert_eq!(r.total, 4);
}
