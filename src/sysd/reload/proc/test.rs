//! SelfReloader 集成测试 (自定义命令).

use crate::{conf::limit::SysdLimit, sysd::reload::Reloader};

use super::{SelfReloader, self_cmd};

/// 一个 运行时间明显超过硬超时 (2 秒) 的 命令.
#[cfg(not(target_os = "windows"))]
fn long_cmd() -> Vec<String> {
    vec!["sleep".to_string(), "30".to_string()]
}

/// Windows 版本: 使用 ping 保持运行.
#[cfg(target_os = "windows")]
fn long_cmd() -> Vec<String> {
    vec![
        "ping".to_string(),
        "-n".to_string(),
        "60".to_string(),
        "127.0.0.1".to_string(),
    ]
}

#[test]
fn self_cmd_is_reload_argv() {
    let argv = self_cmd();
    assert_eq!(argv.len(), 2);
    assert_eq!(argv[1], "--reload");
}

#[tokio::test]
async fn reload_times_out_and_kills_child() {
    let r = SelfReloader::new(long_cmd, &SysdLimit::default());
    let e = match r.reload(None, "unused").await {
        Ok(_) => panic!("reload should time out"),
        Err(e) => e,
    };
    assert!(e.to_string().contains("timed out"));
}
