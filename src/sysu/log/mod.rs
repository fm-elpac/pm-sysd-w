//! pm-sysd-log 使用的 journalctl 风格输出.
//!
//! 初版实现 journalctl 的 short 格式:
//! `MMM DD HH:MM:SS hostname unit: message`.

use chrono::{DateTime, Local};

use crate::{jsonl::SdResLogItem, pa::hostname};

/// 打印日志 (模仿 journalctl 的 short 格式)
pub fn print_logs(logs: &[SdResLogItem]) {
    for e in logs {
        let program = e.unit.as_deref().unwrap_or("pm-sysd-w");
        println!("{} {} {}: {}", format_ts(&e.ts), hostname(), program, e.msg);
    }
}

/// 时间戳 (UTC ISO 8601) -> 本地时间 `MMM DD HH:MM:SS`
fn format_ts(ts: &str) -> String {
    match DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => dt
            .with_timezone(&Local)
            .format("%b %d %H:%M:%S")
            .to_string(),
        Err(_) => ts.to_string(),
    }
}

#[cfg(test)]
mod test;
