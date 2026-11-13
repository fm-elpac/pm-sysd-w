//! cli::log 模块测试.

use super::{Instance, format_ts, parse_args};

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_args_default() {
    // 未显式指定实例时, parse_args 返回 None; 实际选择在 resolve_instance 中完成.
    let a = parse_args(&s(&[])).unwrap();
    assert_eq!(a.instance, None);
    assert!(a.unit.is_none());
}

#[test]
fn parse_args_unit() {
    let a = parse_args(&s(&["-eu", "a.service"])).unwrap();
    assert_eq!(a.instance, None);
    assert_eq!(a.unit.as_deref(), Some("a.service"));
}

#[test]
fn parse_args_unit_combined() {
    let a = parse_args(&s(&["-eua.service"])).unwrap();
    assert_eq!(a.unit.as_deref(), Some("a.service"));
}

#[test]
fn parse_args_unit_separate() {
    let a = parse_args(&s(&["-e", "-u", "a.service"])).unwrap();
    assert_eq!(a.unit.as_deref(), Some("a.service"));
}

#[test]
fn parse_args_unit_eq() {
    let a = parse_args(&s(&["--unit=a.service"])).unwrap();
    assert_eq!(a.unit.as_deref(), Some("a.service"));

    let a = parse_args(&s(&["-u=a.service"])).unwrap();
    assert_eq!(a.unit.as_deref(), Some("a.service"));
}

#[test]
fn parse_args_unit_completes_service_suffix() {
    let a = parse_args(&s(&["-u", "a"])).unwrap();
    assert_eq!(a.unit.as_deref(), Some("a.service"));

    let a = parse_args(&s(&["--unit=a"])).unwrap();
    assert_eq!(a.unit.as_deref(), Some("a.service"));

    let a = parse_args(&s(&["-eua"])).unwrap();
    assert_eq!(a.unit.as_deref(), Some("a.service"));
}

#[test]
fn parse_args_flags_ignored() {
    let a = parse_args(&s(&["-e", "-x", "--pager-end", "--catalog"])).unwrap();
    assert!(a.unit.is_none());
}

#[test]
fn parse_args_machine() {
    let a = parse_args(&s(&["--machine=dev", "-u", "a.service"])).unwrap();
    assert_eq!(a.instance, Some(Instance::Machine("dev".to_string())));
    assert_eq!(a.unit.as_deref(), Some("a.service"));
}

#[test]
fn parse_args_machine_eq() {
    let a = parse_args(&s(&["-M=dev"])).unwrap();
    assert_eq!(a.instance, Some(Instance::Machine("dev".to_string())));
}

#[test]
fn parse_args_machine_separate() {
    let a = parse_args(&s(&["-M", "dev"])).unwrap();
    assert_eq!(a.instance, Some(Instance::Machine("dev".to_string())));
}

#[test]
fn parse_args_machine_missing() {
    assert!(parse_args(&s(&["-M"])).is_err());
}

#[test]
fn parse_args_unit_missing() {
    assert!(parse_args(&s(&["-u"])).is_err());
}

#[test]
fn parse_args_unknown() {
    assert!(parse_args(&s(&["--bogus"])).is_err());
}

#[test]
fn parse_args_instance_conflict() {
    assert!(parse_args(&s(&["--system", "--user"])).is_err());
    assert!(parse_args(&s(&["--system", "-M", "dev"])).is_err());
}

#[test]
fn parse_args_unexpected_positional() {
    assert!(parse_args(&s(&["foo"])).is_err());
}

#[test]
fn format_ts_fallback() {
    assert_eq!(format_ts("not-a-time"), "not-a-time");
}

#[test]
fn format_ts_valid() {
    let input = "2026-01-01T00:00:00Z";
    let out = format_ts(input);
    assert!(!out.is_empty());
    assert_ne!(out, input);
}
