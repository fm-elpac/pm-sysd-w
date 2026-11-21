//! db 模块测试 (DbLog: 日志相关操作).

use crate::test::TempDir;

use super::super::util::Db;
use super::{DbLog, LogMsg, LogPos, log_msg_size};

fn tmp_db(dir: &TempDir, name: &str) -> String {
    dir.path().join(name).to_string_lossy().into_owned()
}

fn open_log(dir: &TempDir, name: &str) -> DbLog {
    let db = Db::open(&tmp_db(dir, name)).unwrap();
    DbLog::new(db)
}

/// 追加一条日志 (测试辅助, 避免在 mod.rs 中保留测试专用代码).
fn append(log: &DbLog, ts: &str, level: &str, unit: Option<&str>, msg: &str) {
    let m = LogMsg {
        ts: ts.to_string(),
        level: level.to_string(),
        unit: unit.map(|s| s.to_string()),
        msg: msg.to_string(),
    };
    log.log_append_batch(&[m]).unwrap();
}

#[test]
fn log_append_filter() {
    let dir = TempDir::new("db");
    let log = open_log(&dir, "a.db");

    append(&log, "2026-11-10T00:00:00.000Z", "info", None, "hello");
    append(
        &log,
        "2026-11-10T00:00:01.000Z",
        "error",
        Some("a.service"),
        "boom",
    );

    // 按 id 倒序返回.
    let all = log
        .log_page(None, None, false, LogPos::End(0), 10, usize::MAX)
        .unwrap();
    assert_eq!(all.total, 2);
    assert_eq!(all.entries.len(), 2);
    assert_eq!(all.entries[0].msg, "boom");
    assert_eq!(all.entries[1].msg, "hello");

    let one = log
        .log_page(
            None,
            Some("a.service"),
            false,
            LogPos::End(0),
            10,
            usize::MAX,
        )
        .unwrap();
    assert_eq!(one.total, 1);
    assert_eq!(one.entries.len(), 1);
    assert_eq!(one.entries[0].msg, "boom");

    let err = log
        .log_page(Some("error"), None, false, LogPos::End(0), 10, usize::MAX)
        .unwrap();
    assert_eq!(err.total, 1);
    assert_eq!(err.entries.len(), 1);
    assert_eq!(err.entries[0].msg, "boom");
}

#[test]
fn log_page_limit_and_order() {
    let dir = TempDir::new("db");
    let log = open_log(&dir, "a.db");

    for i in 0..5 {
        append(
            &log,
            &format!("2026-11-10T00:00:0{i}.000Z"),
            "info",
            None,
            &format!("m{i}"),
        );
    }

    let page = log
        .log_page(None, None, false, LogPos::End(0), 3, usize::MAX)
        .unwrap();
    assert_eq!(page.total, 5);
    assert_eq!(page.entries.len(), 3);
    // 按 id 倒序返回, 只取最新 3 条.
    assert_eq!(page.entries[0].msg, "m4");
    assert_eq!(page.entries[2].msg, "m2");
}

#[test]
fn log_delete_before() {
    let dir = TempDir::new("db");
    let log = open_log(&dir, "a.db");

    append(&log, "2026-11-10T00:00:00.000Z", "info", None, "old");
    append(&log, "2026-11-11T00:00:00.000Z", "info", None, "new");

    let n = log.log_delete_before("2026-11-11T00:00:00.000Z").unwrap();
    assert_eq!(n, 1);

    let all = log
        .log_page(None, None, false, LogPos::End(0), 10, usize::MAX)
        .unwrap();
    assert_eq!(all.total, 1);
    assert_eq!(all.entries.len(), 1);
    assert_eq!(all.entries[0].msg, "new");
}

#[test]
fn log_append_batch_writes_all() {
    let dir = TempDir::new("db");
    let log = open_log(&dir, "b.db");

    let logs = vec![
        LogMsg {
            ts: "2026-11-10T00:00:00.000Z".to_string(),
            level: "info".to_string(),
            unit: None,
            msg: "one".to_string(),
        },
        LogMsg {
            ts: "2026-11-10T00:00:01.000Z".to_string(),
            level: "error".to_string(),
            unit: Some("a.service".to_string()),
            msg: "two".to_string(),
        },
    ];
    log.log_append_batch(&logs).unwrap();

    let all = log
        .log_page(None, None, false, LogPos::End(0), 10, usize::MAX)
        .unwrap();
    assert_eq!(all.total, 2);
    assert_eq!(all.entries.len(), 2);
    assert_eq!(all.entries[0].msg, "two");
    assert_eq!(all.entries[1].msg, "one");
}

#[test]
fn log_msg_size_sums_fields() {
    let m = LogMsg {
        ts: "ab".to_string(),
        level: "cd".to_string(),
        unit: Some("ef".to_string()),
        msg: "gh".to_string(),
    };
    assert_eq!(log_msg_size(&m), 8);

    let m = LogMsg {
        ts: "a".to_string(),
        level: "b".to_string(),
        unit: None,
        msg: "c".to_string(),
    };
    assert_eq!(log_msg_size(&m), 3);
}
