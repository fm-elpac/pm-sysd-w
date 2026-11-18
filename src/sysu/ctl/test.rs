//! pm-sysd-ctl 输出格式测试.

use chrono::Utc;

use super::{format_relative, format_status_since};

#[test]
fn relative_now() {
    assert_eq!(format_relative(Utc::now().timestamp_millis()), "now");
}

#[test]
fn relative_seconds() {
    let ms = Utc::now().timestamp_millis() - 90_000;
    assert_eq!(format_relative(ms), "1min 30s ago");
}

#[test]
fn relative_minutes() {
    let ms = Utc::now().timestamp_millis() - 5 * 60_000;
    assert_eq!(format_relative(ms), "5min ago");
}

#[test]
fn status_since_suffix() {
    let ms = Utc::now().timestamp_millis() - 90_000;
    let out = format_status_since(ms);
    assert!(out.starts_with(" since "), "{}", out);
    assert!(out.ends_with("; 1min 30s ago"), "{}", out);
}
