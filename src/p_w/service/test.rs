//! p_w::service 测试.

use super::{StopEvent, to_wide};

#[test]
fn to_wide_null_terminated() {
    let w = to_wide("pm_sysd_w_user");
    assert_eq!(w.len(), "pm_sysd_w_user".len() + 1);
    assert_eq!(w.last(), Some(&0));
}

#[test]
fn stop_event_request_flag() {
    let e = StopEvent::new();
    assert!(!e.is_requested());
    e.request();
    assert!(e.is_requested());
}
