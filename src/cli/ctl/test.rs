//! cli::ctl 模块测试.

use crate::{
    cli::resolve_instance_with,
    jsonl::{SdReq, SdReqUnit, SdReqUnitOp, SdReqUnitOpRO, SdReqUnitRO, SdReqUnitd},
};

use super::{Instance, build_command, parse_args};

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_args_start() {
    let a = parse_args(&s(&["--machine=dev", "start", "a.service"])).unwrap();
    assert_eq!(a.instance, Some(Instance::Machine("dev".to_string())));
    assert_eq!(a.command, "start");
    assert_eq!(a.units, vec!["a.service"]);
}

#[test]
fn parse_args_no_explicit_instance() {
    // 未显式指定实例时, parse_args 返回 None; 实际选择在 resolve_instance 中完成.
    let a = parse_args(&s(&["list-units"])).unwrap();
    assert_eq!(a.instance, None);
    assert_eq!(a.command, "list-units");
}

#[test]
fn parse_args_user() {
    let a = parse_args(&s(&["--user", "list-units"])).unwrap();
    assert_eq!(a.instance, Some(Instance::User));
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
    assert_eq!(a.instance, Some(Instance::Machine("dev".to_string())));
}

#[test]
fn parse_args_machine_eq() {
    let a = parse_args(&s(&["-M=dev", "list-units"])).unwrap();
    assert_eq!(a.instance, Some(Instance::Machine("dev".to_string())));
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
fn parse_args_instance_conflict() {
    assert!(parse_args(&s(&["--system", "--user", "list-units"])).is_err());
    assert!(parse_args(&s(&["--system", "-M", "dev", "list-units"])).is_err());
}

#[test]
fn build_command_start() {
    let c = build_command("start", &s(&["a.service"]), false).unwrap();
    assert!(matches!(
        c,
        SdReq::Unit(SdReqUnit { name, op: SdReqUnitOp::Start }) if name == "a.service"
    ));
}

#[test]
fn build_command_enable_now() {
    let c = build_command("enable", &s(&["a.service"]), true).unwrap();
    assert!(matches!(
        c,
        SdReq::Unit(SdReqUnit { name, op: SdReqUnitOp::Enable { now: true } }) if name == "a.service"
    ));
}

#[test]
fn build_command_reset_failed_no_unit() {
    let c = build_command("reset-failed", &[], false).unwrap();
    assert!(matches!(
        c,
        SdReq::Unit(SdReqUnit { name, op: SdReqUnitOp::ResetFailed }) if name.is_empty()
    ));
}

#[test]
fn build_command_reset_failed_one_unit() {
    let c = build_command("reset-failed", &s(&["a.service"]), false).unwrap();
    assert!(matches!(
        c,
        SdReq::Unit(SdReqUnit { name, op: SdReqUnitOp::ResetFailed }) if name == "a.service"
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
        SdReq::UnitD(SdReqUnitd::SetDefault { target }) if target == "default.target"
    ));
}

#[test]
fn build_command_set_default_no_completion() {
    // set-default 的 target 参数 原样透传, 不自动补 `.service`.
    let c = build_command("set-default", &s(&["default"]), false).unwrap();
    assert!(matches!(
        c,
        SdReq::UnitD(SdReqUnitd::SetDefault { target }) if target == "default"
    ));
}

#[test]
fn build_command_completes_service_suffix() {
    let c = build_command("start", &s(&["a"]), false).unwrap();
    assert!(matches!(
        c,
        SdReq::Unit(SdReqUnit { name, op: SdReqUnitOp::Start }) if name == "a.service"
    ));

    let c = build_command("status", &s(&["a.service"]), false).unwrap();
    assert!(matches!(
        c,
        SdReq::UnitRO(SdReqUnitRO { name, op: SdReqUnitOpRO::Status }) if name == "a.service"
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

#[test]
fn build_command_import_environment_not_implemented() {
    assert!(build_command("import-environment", &[], false).is_err());
}

#[test]
fn resolve_instance_explicit() {
    let r = resolve_instance_with(|_| false, Some(Instance::Machine("dev".to_string()))).unwrap();
    assert_eq!(r, Instance::Machine("dev".to_string()));
}

#[test]
fn resolve_instance_prefer_system() {
    let system = Instance::System.transport();
    let r = resolve_instance_with(|p| p == system.as_str(), None).unwrap();
    assert_eq!(r, Instance::System);
}

#[test]
fn resolve_instance_fallback_user() {
    let user = Instance::User.transport();
    let r = resolve_instance_with(|p| p == user.as_str(), None).unwrap();
    assert_eq!(r, Instance::User);
}

#[test]
fn resolve_instance_none() {
    assert!(resolve_instance_with(|_| false, None).is_err());
}
