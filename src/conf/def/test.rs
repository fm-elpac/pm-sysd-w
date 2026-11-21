//! conf::def 模块测试.

use super::{RestartPolicy, ServiceType};

#[test]
fn enum_as_str() {
    assert_eq!(ServiceType::Simple.as_str(), "simple");
    assert_eq!(ServiceType::Oneshot.as_str(), "oneshot");
    assert_eq!(RestartPolicy::No.as_str(), "no");
    assert_eq!(RestartPolicy::Always.as_str(), "always");
    assert_eq!(RestartPolicy::OnFailure.as_str(), "on-failure");
}
