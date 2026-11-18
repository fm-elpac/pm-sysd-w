//! sysu::log 模块测试.

use super::format_ts;

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
