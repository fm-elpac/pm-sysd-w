//! pa 模块测试.

use super::*;

#[test]
fn split_cmd_basic() {
    assert_eq!(split_cmd("path/to/bin --arg"), vec!["path/to/bin", "--arg"]);
}

#[test]
fn split_cmd_quotes() {
    assert_eq!(
        split_cmd("bin \"hello world\" x"),
        vec!["bin", "hello world", "x"]
    );
}

#[test]
fn split_cmd_empty() {
    assert_eq!(split_cmd(""), Vec::<String>::new());
}

#[test]
fn split_cmd_backslash_escape() {
    assert_eq!(split_cmd("bin a\\ b"), vec!["bin", "a b"]);
}

#[test]
fn split_cmd_backslash_at_end() {
    assert_eq!(split_cmd("bin a\\"), vec!["bin", "a\\"]);
}

#[test]
fn split_cmd_extra_whitespace() {
    assert_eq!(split_cmd("  bin   x  "), vec!["bin", "x"]);
}

#[test]
fn instance_machine_name() {
    let i = Instance::Machine("dev".to_string());
    assert_eq!(i.name(), "pm_sysd_w_m_dev");
}

#[test]
fn instance_system_name() {
    assert_eq!(Instance::System.name(), "pm_sysd_w_system_default");
}

#[test]
fn instance_user_name_prefix() {
    assert!(Instance::User.name().starts_with("pm_sysd_w_user_"));
}

#[test]
fn instance_transport() {
    let i = Instance::Machine("dev".to_string());
    #[cfg(target_os = "linux")]
    assert_eq!(i.transport(), "./pm_sysd_w_m_dev.sock");
    #[cfg(target_os = "windows")]
    assert_eq!(i.transport(), "pm_sysd_w_m_dev");
}

#[test]
fn resolve_path_relative() {
    let base = Path::new("/a/b");
    assert_eq!(resolve_path(base, "c.db"), PathBuf::from("/a/b/c.db"));
    assert_eq!(resolve_path(base, "/c.db"), PathBuf::from("/c.db"));
}
