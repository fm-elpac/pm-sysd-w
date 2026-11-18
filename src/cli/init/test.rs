//! cli::init 模块测试.

use super::{Instance, parse_args};

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn parse_args_machine() {
    let a = parse_args(&s(&["--machine=dev", "--conf=/x/c.toml"])).unwrap();
    assert_eq!(a.instance, Instance::Machine("dev".to_string()));
    assert_eq!(a.conf, "/x/c.toml");
}

#[test]
fn parse_args_system() {
    let a = parse_args(&s(&["--system", "--conf=/x/c.toml"])).unwrap();
    assert_eq!(a.instance, Instance::System);
}

#[test]
fn parse_args_user() {
    let a = parse_args(&s(&["--user", "--conf=/x/c.toml"])).unwrap();
    assert_eq!(a.instance, Instance::User);
}

#[test]
fn parse_args_missing_instance() {
    assert!(parse_args(&s(&["--conf=/x/c.toml"])).is_err());
}

#[test]
fn parse_args_missing_conf() {
    assert!(parse_args(&s(&["--system"])).is_err());
}

#[test]
fn parse_args_unknown() {
    assert!(parse_args(&s(&["--bogus"])).is_err());
}

#[test]
fn parse_args_empty_machine() {
    assert!(parse_args(&s(&["--machine=", "--conf=/x/c.toml"])).is_err());
}

#[test]
fn parse_args_machine_charset() {
    let a = parse_args(&s(&["--machine=AZaz09_-", "--conf=/x/c.toml"])).unwrap();
    assert_eq!(a.instance, Instance::Machine("AZaz09_-".to_string()));
}

#[test]
fn parse_args_machine_invalid_char() {
    assert!(parse_args(&s(&["--machine=bad.name", "--conf=/x/c.toml"])).is_err());
    assert!(parse_args(&s(&["--machine=bad/name", "--conf=/x/c.toml"])).is_err());
    assert!(parse_args(&s(&["--machine=坏", "--conf=/x/c.toml"])).is_err());
}

#[test]
fn parse_args_machine_max_len() {
    let name = "a".repeat(100);
    let arg = format!("--machine={}", name);
    let a = parse_args(&s(&[arg.as_str(), "--conf=/x/c.toml"])).unwrap();
    assert_eq!(a.instance, Instance::Machine(name));
}

#[test]
fn parse_args_machine_too_long() {
    let name = "a".repeat(101);
    let arg = format!("--machine={}", name);
    assert!(parse_args(&s(&[arg.as_str(), "--conf=/x/c.toml"])).is_err());
}

#[test]
fn parse_args_empty_conf() {
    assert!(parse_args(&s(&["--system", "--conf="])).is_err());
}

#[test]
fn parse_args_instance_conflict() {
    assert!(parse_args(&s(&["--system", "--user", "--conf=/x/c.toml"])).is_err());
    assert!(parse_args(&s(&["--system", "--machine=dev", "--conf=/x/c.toml"])).is_err());
}
