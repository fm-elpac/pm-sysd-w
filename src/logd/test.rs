//! logd 模块测试.

use std::sync::Arc;

use super::*;
use crate::testutil::TempDir;

fn tmp_db(dir: &TempDir) -> String {
    dir.path().join("a.db").to_string_lossy().into_owned()
}

#[test]
fn write_and_recent() {
    let dir = TempDir::new("logd");
    let path = tmp_db(&dir);
    let db = Arc::new(Db::open(&path).unwrap());
    let mut logd = Logd::new(db);

    logd.write("hello").unwrap();
    logd.write_with("error", Some("a.service"), "boom").unwrap();
    logd.flush().unwrap();

    let all = logd.recent(None, 10).unwrap();
    assert_eq!(all.len(), 2);
    assert!(all[0].contains("hello"));

    let one = logd.recent(Some("a.service"), 10).unwrap();
    assert_eq!(one.len(), 1);
    assert!(one[0].contains("boom"));
}

#[test]
fn recent_entries() {
    let dir = TempDir::new("logd");
    let path = tmp_db(&dir);
    let db = Arc::new(Db::open(&path).unwrap());
    let logd = Logd::new(db);

    logd.write_with("info", None, "m").unwrap();
    logd.write_with("error", Some("a.service"), "e").unwrap();
    logd.flush().unwrap();

    let entries = logd.recent_entries(Some("a.service"), 10).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].level, "error");
    assert_eq!(entries[0].unit.as_deref(), Some("a.service"));
    assert_eq!(entries[0].msg, "e");
}
