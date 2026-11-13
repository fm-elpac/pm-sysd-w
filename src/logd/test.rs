//! logd 模块测试.

use crate::{db::DbClient, test::TempDir};

use super::{Logd, split_msg};

fn tmp_db(dir: &TempDir) -> String {
    dir.path().join("a.db").to_string_lossy().into_owned()
}

#[tokio::test]
async fn write_and_recent() {
    let dir = TempDir::new("logd");
    let path = tmp_db(&dir);
    let db = DbClient::open(&path).unwrap();
    let mut logd = Logd::new(db);

    logd.write("hello").unwrap();
    logd.write_with("error", Some("a.service"), "boom").unwrap();
    logd.flush().unwrap();

    let all = logd.recent(None, 10).await.unwrap();
    assert_eq!(all.len(), 2);
    assert!(all[0].contains("hello"));

    let one = logd.recent(Some("a.service"), 10).await.unwrap();
    assert_eq!(one.len(), 1);
    assert!(one[0].contains("boom"));
}

#[tokio::test]
async fn recent_entries() {
    let dir = TempDir::new("logd");
    let path = tmp_db(&dir);
    let db = DbClient::open(&path).unwrap();
    let logd = Logd::new(db);

    logd.write_with("info", None, "m").unwrap();
    logd.write_with("error", Some("a.service"), "e").unwrap();
    logd.flush().unwrap();

    let entries = logd.recent_entries(Some("a.service"), 10).await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].level, "error");
    assert_eq!(entries[0].unit.as_deref(), Some("a.service"));
    assert_eq!(entries[0].msg, "e");
}

#[test]
fn split_msg_chunks() {
    let msg = "abcdef";
    assert_eq!(split_msg(msg, 2), vec!["ab", "cd", "ef"]);
    assert_eq!(split_msg(msg, 10), vec!["abcdef"]);

    // 在 UTF-8 字符边界 拆分 (不产生 panic).
    let m = "你好世界";
    let chunks = split_msg(m, 3);
    assert_eq!(chunks.concat(), m);
}

#[tokio::test]
async fn page_entries_pagination() {
    let dir = TempDir::new("logd");
    let path = tmp_db(&dir);
    let db = DbClient::open(&path).unwrap();
    let logd = Logd::new(db);

    for i in 0..5 {
        logd.write_with("info", None, &format!("m{}", i)).unwrap();
    }
    logd.flush().unwrap();

    // 第一页: 最新 2 条.
    let p1 = logd.page_entries(None, 2, None).await.unwrap();
    assert_eq!(p1.total, 5);
    assert_eq!(p1.entries.len(), 2);
    assert_eq!(p1.entries[0].msg, "m3");
    assert_eq!(p1.entries[1].msg, "m4");
    let c1 = p1.cursor.expect("next cursor");

    // 第二页: 继续读取.
    let p2 = logd.page_entries(None, 2, Some(c1)).await.unwrap();
    assert_eq!(p2.total, 5);
    assert_eq!(p2.entries.len(), 2);
    assert_eq!(p2.entries[0].msg, "m1");
    assert_eq!(p2.entries[1].msg, "m2");
    let c2 = p2.cursor.expect("next cursor");

    // 第三页: 最后 1 条, 无下一页.
    let p3 = logd.page_entries(None, 2, Some(c2)).await.unwrap();
    assert_eq!(p3.entries.len(), 1);
    assert_eq!(p3.entries[0].msg, "m0");
    assert_eq!(p3.cursor, None);
}
