//! sysd 模块测试: 状态枚举的单元测试

use super::{ServiceSta, TargetSta, TimerSta, UnitSubSta};

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
