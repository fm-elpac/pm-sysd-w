//! db 模块测试.

use super::*;
use crate::testutil::TempDir;

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
    assert!(db.unit_get_enabled("a.service").unwrap());
    assert!(!db.unit_get_enabled("b.service").unwrap());

    db.unit_set_masked("a.service", true).unwrap();
    assert!(db.unit_get_masked("a.service").unwrap());
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
        assert!(db.unit_get_enabled("a.service").unwrap());
    }
}

#[test]
fn log_append_recent() {
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

    let all = db.log_recent(None, 10).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].msg, "hello");
    assert_eq!(all[1].msg, "boom");

    let one = db.log_recent(Some("a.service"), 10).unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].msg, "boom");
}

#[test]
fn log_recent_limit_and_order() {
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

    let recent = db.log_recent(None, 3).unwrap();
    assert_eq!(recent.len(), 3);
    assert_eq!(recent[0].msg, "m2");
    assert_eq!(recent[2].msg, "m4");
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
