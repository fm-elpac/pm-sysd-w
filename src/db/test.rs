//! db 模块测试.

use crate::test::TempDir;

use super::{Db, PROGRAM_NAME};

fn tmp_db(dir: &TempDir, name: &str) -> String {
    dir.path().join(name).to_string_lossy().into_owned()
}

#[test]
fn open_and_persist() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.meta_get("program").unwrap().as_deref(),
        Some(PROGRAM_NAME)
    );
    assert_eq!(db.meta_get("schema_version").unwrap().as_deref(), Some("1"));

    db.unit_set_enabled("a.service", true).unwrap();
    assert_eq!(db.unit_list_enabled().unwrap(), vec!["a.service"]);

    db.unit_set_masked("a.service", true).unwrap();
    assert_eq!(db.unit_list_masked().unwrap(), vec!["a.service"]);
}

#[test]
fn reopen_keeps_state() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    {
        let db = Db::open(&path).unwrap();
        db.unit_set_enabled("a.service", true).unwrap();
    }
    {
        let db = Db::open(&path).unwrap();
        assert_eq!(db.unit_list_enabled().unwrap(), vec!["a.service"]);
    }
}

#[test]
fn log_append_filter() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    let db = Db::open(&path).unwrap();
    db.log_append("2026-11-10T00:00:00.000Z", "info", None, "hello")
        .unwrap();
    db.log_append(
        "2026-11-10T00:00:01.000Z",
        "error",
        Some("a.service"),
        "boom",
    )
    .unwrap();

    // 按 id 倒序返回.
    let all = db.log_page_raw(None, None, 10, None, usize::MAX).unwrap();
    assert_eq!(all.entries.len(), 2);
    assert_eq!(all.entries[0].msg, "boom");
    assert_eq!(all.entries[1].msg, "hello");

    let one = db
        .log_page_raw(None, Some("a.service"), 10, None, usize::MAX)
        .unwrap();
    assert_eq!(one.entries.len(), 1);
    assert_eq!(one.entries[0].msg, "boom");

    let err = db
        .log_page_raw(Some("error"), None, 10, None, usize::MAX)
        .unwrap();
    assert_eq!(err.entries.len(), 1);
    assert_eq!(err.entries[0].msg, "boom");
}

#[test]
fn log_page_raw_limit_and_order() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    let db = Db::open(&path).unwrap();
    for i in 0..5 {
        db.log_append(
            &format!("2026-11-10T00:00:0{i}.000Z"),
            "info",
            None,
            &format!("m{i}"),
        )
        .unwrap();
    }

    let page = db.log_page_raw(None, None, 3, None, usize::MAX).unwrap();
    assert_eq!(page.entries.len(), 3);
    // 按 id 倒序返回, 只取最新 3 条.
    assert_eq!(page.entries[0].msg, "m4");
    assert_eq!(page.entries[2].msg, "m2");
}

#[test]
fn new_db_meta() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    let db = Db::open(&path).unwrap();
    assert_eq!(
        db.meta_get("created_by_version").unwrap().as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        db.meta_get("last_upgrade_version").unwrap().as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );

    let created = db.meta_get("created_at").unwrap();
    let updated = db.meta_get("updated_at").unwrap();
    assert!(created.is_some());
    assert_eq!(created, updated);
}

#[test]
fn open_program_mismatch() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    {
        let db = Db::open(&path).unwrap();
        db.meta_set("program", "other").unwrap();
    }

    let e = Db::open(&path).unwrap_err();
    assert!(e.to_string().contains("other"));
}

#[test]
fn open_newer_schema() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    {
        let db = Db::open(&path).unwrap();
        db.meta_set("schema_version", "999").unwrap();
    }

    let e = Db::open(&path).unwrap_err();
    assert!(e.to_string().contains("newer"));
}

#[test]
fn open_older_schema_upgrades() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    {
        let db = Db::open(&path).unwrap();
        db.meta_set("schema_version", "0").unwrap();
    }

    let db = Db::open(&path).unwrap();
    assert_eq!(db.meta_get("schema_version").unwrap().as_deref(), Some("1"));
}

#[test]
fn fresh_db_has_indexes() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    let db = Db::open(&path).unwrap();
    let conn = db.conn.lock().unwrap();
    assert_eq!(index_exists(&conn, "idx_log_unit"), 1);
    assert_eq!(index_exists(&conn, "idx_log_level"), 1);
    assert_eq!(index_exists(&conn, "idx_log_ts"), 1);
}

#[test]
fn log_delete_before() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    let db = Db::open(&path).unwrap();
    db.log_append("2026-11-10T00:00:00.000Z", "info", None, "old")
        .unwrap();
    db.log_append("2026-11-11T00:00:00.000Z", "info", None, "new")
        .unwrap();

    let n = db.log_delete_before("2026-11-11T00:00:00.000Z").unwrap();
    assert_eq!(n, 1);

    let all = db.log_page_raw(None, None, 10, None, usize::MAX).unwrap();
    assert_eq!(all.entries.len(), 1);
    assert_eq!(all.entries[0].msg, "new");
}

#[test]
fn vacuum_ok() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    let db = Db::open(&path).unwrap();
    db.unit_set_enabled("a.service", true).unwrap();
    db.vacuum().unwrap();
}

/// 检查索引是否存在 (0/1).
fn index_exists(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
        rusqlite::params![name],
        |row| row.get(0),
    )
    .unwrap()
}
