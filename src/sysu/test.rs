//! sysu 模块测试: 状态枚举与打印格式化.

use super::{DaemonSta, ServiceSta, TargetSta, TimerSta, UnitSubSta, reached_target};

#[test]
fn reached_target_format() {
    assert_eq!(
        reached_target("default.target"),
        "[  OK  ] Reached target default.target."
    );
    assert_eq!(
        reached_target("exit.target"),
        "[  OK  ] Reached target exit.target."
    );
}

#[test]
fn daemon_sta_display_and_exit_code() {
    let cases = [
        (DaemonSta::Initializing, "initializing", 1),
        (DaemonSta::Starting, "starting", 1),
        (DaemonSta::Running, "running", 0),
        (DaemonSta::Degraded, "degraded", 1),
        (DaemonSta::Maintenance, "maintenance", 1),
        (DaemonSta::Stopping, "stopping", 1),
        (DaemonSta::Offline, "offline", 1),
        (DaemonSta::Unknown, "unknown", 1),
    ];
    for (sta, text, code) in cases {
        assert_eq!(sta.as_str(), text);
        assert_eq!(sta.to_string(), text);
        assert_eq!(sta.exit_code(), code);
    }
}

#[test]
fn unit_sub_sta_display() {
    assert_eq!(
        UnitSubSta::Service(ServiceSta::Exited).to_string(),
        "active(exited)"
    );
    assert_eq!(
        UnitSubSta::Service(ServiceSta::Running).to_string(),
        "active(running)"
    );
    assert_eq!(
        UnitSubSta::Service(ServiceSta::Dead).to_string(),
        "inactive(dead)"
    );
}

#[test]
fn unit_sub_sta_serde_preserves_type() {
    let cases = [
        UnitSubSta::Service(ServiceSta::Running),
        UnitSubSta::Timer(TimerSta::Running),
        UnitSubSta::Target(TargetSta::Active),
    ];
    for c in cases {
        let line = serde_json::to_string(&c).unwrap();
        let c2: UnitSubSta = serde_json::from_str(&line).unwrap();
        assert_eq!(c, c2);
    }
}
