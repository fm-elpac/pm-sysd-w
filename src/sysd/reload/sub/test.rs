//! reload::sub 模块测试 (--reload 核心逻辑).

use crate::{conf::limit::SysdLimit, test::TempDir};

use super::run_reload_conf;

#[test]
fn reload_conf_serializes_conf() {
    let dir = TempDir::new("reload_sub");
    let path = dir.path().join("c.toml");
    std::fs::write(&path, "pm_tc = 1\n").unwrap();

    let json = run_reload_conf(&path.to_string_lossy(), &SysdLimit::default()).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert!(v.get("units").unwrap().as_array().unwrap().is_empty());
    assert!(v.get("warnings").unwrap().as_array().unwrap().is_empty());
    assert!(
        v.get("db_path")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("pm_sysd_w_v1.db")
    );
}

#[test]
fn reload_conf_empty_path() {
    assert_eq!(
        run_reload_conf("", &SysdLimit::default()).unwrap_err(),
        "PM_SYSD_CONF is empty"
    );
}

#[test]
fn reload_conf_missing_file() {
    let dir = TempDir::new("reload_sub_missing");
    let path = dir.path().join("missing.toml");
    assert!(run_reload_conf(&path.to_string_lossy(), &SysdLimit::default()).is_err());
}

#[test]
fn reload_conf_output_too_large() {
    let dir = TempDir::new("reload_sub_large");
    let path = dir.path().join("c.toml");

    // 通过 pm-tc 的 `[n:NAME]` 指数展开, 让 `SYSD_DB` 膨胀为约 12MB 的路径.
    // `SYSD_DB` 不是 `Unit.*` / `Service.*` 等受支持 key, 不会被 conf 的单值
    // 大小限制拦截, 因此 load_conf 能成功, 直到序列化 JSON 时才触发输出大小限制.
    let text = r#"pm_tc = 1

[env]
a = "[:a][:a][:a][:a]"
SYSD_DB = "[11:a]"
"#;
    std::fs::write(&path, text).unwrap();

    let e = run_reload_conf(&path.to_string_lossy(), &SysdLimit::default()).unwrap_err();
    assert!(e.contains("config JSON output exceeds"));
}
