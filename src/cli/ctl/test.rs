//! cli::ctl 模块测试.

use super::*;

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_args_start() {
    let a = parse_args(&s(&["--machine=dev", "start", "a.service"])).unwrap();
    assert_eq!(a.instance, Instance::Machine("dev".to_string()));
    assert_eq!(a.command, "start");
    assert_eq!(a.units, vec!["a.service"]);
}

#[test]
fn parse_args_default_system() {
    let a = parse_args(&s(&["list-units"])).unwrap();
    assert_eq!(a.instance, Instance::System);
    assert_eq!(a.command, "list-units");
}

#[test]
fn parse_args_user() {
    let a = parse_args(&s(&["--user", "list-units"])).unwrap();
    assert_eq!(a.instance, Instance::User);
}

#[test]
fn parse_args_now() {
    let a = parse_args(&s(&["enable", "--now", "a.service"])).unwrap();
    assert_eq!(a.command, "enable");
    assert!(a.now);
    assert_eq!(a.units, vec!["a.service"]);
}

#[test]
fn parse_args_machine_separate() {
    let a = parse_args(&s(&["-M", "dev", "list-units"])).unwrap();
    assert_eq!(a.instance, Instance::Machine("dev".to_string()));
}

#[test]
fn parse_args_machine_eq() {
    let a = parse_args(&s(&["-M=dev", "list-units"])).unwrap();
    assert_eq!(a.instance, Instance::Machine("dev".to_string()));
}

#[test]
fn parse_args_machine_missing() {
    assert!(parse_args(&s(&["-M"])).is_err());
}

#[test]
fn parse_args_wait_no_block_ignored() {
    let a = parse_args(&s(&["--wait", "--no-block", "list-units"])).unwrap();
    assert_eq!(a.command, "list-units");
}

#[test]
fn parse_args_unknown_option() {
    assert!(parse_args(&s(&["--bogus"])).is_err());
}

#[test]
fn parse_args_missing_command() {
    assert!(parse_args(&s(&["--system"])).is_err());
}

#[test]
fn build_command_start() {
    let c = build_command("start", &s(&["a.service"]), false).unwrap();
    assert!(matches!(
        c,
        Command::Start { unit } if unit == "a.service"
    ));
}

#[test]
fn build_command_enable_now() {
    let c = build_command("enable", &s(&["a.service"]), true).unwrap();
    assert!(matches!(
        c,
        Command::Enable { unit, now: true } if unit == "a.service"
    ));
}

#[test]
fn build_command_reset_failed_no_unit() {
    let c = build_command("reset-failed", &[], false).unwrap();
    assert!(matches!(c, Command::ResetFailed { unit: None }));
}

#[test]
fn build_command_reset_failed_one_unit() {
    let c = build_command("reset-failed", &s(&["a.service"]), false).unwrap();
    assert!(matches!(
        c,
        Command::ResetFailed { unit: Some(u) } if u == "a.service"
    ));
}

#[test]
fn build_command_reset_failed_too_many() {
    assert!(build_command("reset-failed", &s(&["a.service", "b.service"]), false).is_err());
}

#[test]
fn build_command_set_default() {
    let c = build_command("set-default", &s(&["default.target"]), false).unwrap();
    assert!(matches!(
        c,
        Command::SetDefault { target } if target == "default.target"
    ));
}

#[test]
fn build_command_missing_unit() {
    assert!(build_command("start", &[], false).is_err());
}

#[test]
fn build_command_unknown() {
    assert!(build_command("nope", &[], false).is_err());
}

#[test]
fn build_command_now_invalid() {
    // `--now` 只对 enable/disable 有效.
    assert!(build_command("start", &s(&["a.service"]), true).is_err());
}

#[test]
fn build_command_too_many_units() {
    assert!(build_command("start", &s(&["a.service", "b.service"]), false).is_err());
}

#[test]
fn build_command_single_unit_arity() {
    // 这些命令都要求 恰好 1 个 unit 参数.
    let cmds = [
        "is-active",
        "is-failed",
        "status",
        "show",
        "cat",
        "list-dependencies",
        "start",
        "stop",
        "reload",
        "restart",
        "try-restart",
        "reload-or-restart",
        "try-reload-or-restart",
        "kill",
        "clean",
        "enable",
        "disable",
        "reenable",
        "preset",
        "is-enabled",
        "mask",
        "unmask",
        "set-default",
    ];
    for c in cmds {
        assert!(
            build_command(c, &s(&["a.service"]), false).is_ok(),
            "command `{}` should accept 1 unit",
            c
        );
        assert!(
            build_command(c, &[], false).is_err(),
            "command `{}` should reject 0 units",
            c
        );
        assert!(
            build_command(c, &s(&["a.service", "b.service"]), false).is_err(),
            "command `{}` should reject 2 units",
            c
        );
    }
}

#[test]
fn build_command_no_arg_commands() {
    let cmds = [
        "list-units",
        "list-timers",
        "get-default",
        "show-environment",
        "set-environment",
        "unset-environment",
        "import-environment",
        "daemon-reload",
        "soft-reboot",
        "exit",
    ];
    for c in cmds {
        assert!(
            build_command(c, &[], false).is_ok(),
            "command `{}` should accept 0 args",
            c
        );
        assert!(
            build_command(c, &s(&["x"]), false).is_err(),
            "command `{}` should reject arguments",
            c
        );
    }
}
