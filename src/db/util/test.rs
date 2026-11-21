//! db 模块测试 (Db 打开 / 初始化 / 事务 / schema).

use crate::test::TempDir;

use super::Db;

fn tmp_db(dir: &TempDir, name: &str) -> String {
    dir.path().join(name).to_string_lossy().into_owned()
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
fn fresh_db_no_index_has_no_log_indexes() {
    let dir = TempDir::new("db");
    let path = tmp_db(&dir, "a.db");

    let db = Db::open_log_no_index(&path).unwrap();
    let conn = db.conn.lock().unwrap();
    assert_eq!(index_exists(&conn, "idx_log_unit"), 0);
    assert_eq!(index_exists(&conn, "idx_log_level"), 0);
    assert_eq!(index_exists(&conn, "idx_log_ts"), 0);
    // 日志表仍然存在, 可正常追加写入.
    conn.execute(
        "INSERT INTO log (ts, level, unit, msg) VALUES ('2026-11-20T00:00:00.000Z', 'info', NULL, 'x')",
        [],
    )
    .unwrap();
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
