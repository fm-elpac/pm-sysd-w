//! logd 模块测试.

use std::time::{Duration, Instant};

use crate::{db::DbClient, test::TempDir};

use super::{Logd, proc::read_log_chunk, split_msg, warn::RateLimitState};

fn tmp_db(dir: &TempDir) -> String {
    dir.path().join("a.db").to_string_lossy().into_owned()
}

#[tokio::test]
async fn write_and_page_entries() {
    let dir = TempDir::new("logd");
    let path = tmp_db(&dir);
    let db = DbClient::open(&path).unwrap();
    let mut logd = Logd::new(db);

    logd.write("hello").unwrap();
    logd.write_with("error", Some("a.service"), "boom").unwrap();
    logd.flush().await.unwrap();

    let all = logd.page_entries(None, None, 10, None).await.unwrap();
    assert_eq!(all.total, 2);
    assert_eq!(all.entries.len(), 2);
    assert!(all.entries.iter().any(|e| e.msg == "hello"));

    let one = logd
        .page_entries(None, Some("a.service"), 10, None)
        .await
        .unwrap();
    assert_eq!(one.total, 1);
    assert_eq!(one.entries.len(), 1);
    assert_eq!(one.entries[0].level, "error");
    assert_eq!(one.entries[0].unit.as_deref(), Some("a.service"));
    assert_eq!(one.entries[0].msg, "boom");

    let err = logd
        .page_entries(Some("error"), None, 10, None)
        .await
        .unwrap();
    assert_eq!(err.total, 1);
    assert_eq!(err.entries.len(), 1);
    assert_eq!(err.entries[0].level, "error");
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
async fn read_log_chunk_caps_buffer() {
    use tokio::io::BufReader;

    // 完整一行: 返回整行并消费换行.
    let mut r = BufReader::new(&b"abc\nrest"[..]);
    let (buf, ended) = read_log_chunk(&mut r, 8).await.unwrap().unwrap();
    assert_eq!(buf, b"abc");
    assert!(ended);

    // 超长行: 每次最多返回 max 字节, 不无限增长.
    let mut r = BufReader::new(&b"1234567890"[..]);
    let (buf, ended) = read_log_chunk(&mut r, 4).await.unwrap().unwrap();
    assert_eq!(buf, b"1234");
    assert!(!ended);
    let (buf, ended) = read_log_chunk(&mut r, 4).await.unwrap().unwrap();
    assert_eq!(buf, b"5678");
    assert!(!ended);
    // 最后一段 (无换行).
    let (buf, ended) = read_log_chunk(&mut r, 4).await.unwrap().unwrap();
    assert_eq!(buf, b"90");
    assert!(!ended);
    assert!(read_log_chunk(&mut r, 4).await.unwrap().is_none());
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
    logd.flush().await.unwrap();

    // 第一页: 最新 2 条.
    let p1 = logd.page_entries(None, None, 2, None).await.unwrap();
    assert_eq!(p1.total, 5);
    assert_eq!(p1.entries.len(), 2);
    assert_eq!(p1.entries[0].msg, "m3");
    assert_eq!(p1.entries[1].msg, "m4");
    let c1 = p1.cursor.expect("next cursor");

    // 第二页: 继续读取.
    let p2 = logd.page_entries(None, None, 2, Some(c1)).await.unwrap();
    assert_eq!(p2.total, 5);
    assert_eq!(p2.entries.len(), 2);
    assert_eq!(p2.entries[0].msg, "m1");
    assert_eq!(p2.entries[1].msg, "m2");
    let c2 = p2.cursor.expect("next cursor");

    // 第三页: 最后 1 条, 无下一页.
    let p3 = logd.page_entries(None, None, 2, Some(c2)).await.unwrap();
    assert_eq!(p3.entries.len(), 1);
    assert_eq!(p3.entries[0].msg, "m0");
    assert_eq!(p3.cursor, None);
}

#[test]
fn warn_rate_limit_per_key() {
    let mut st = RateLimitState::default();
    let t0 = Instant::now();

    // 不同 key 独立限速, 不会互相饿死
    assert!(st.process("a", "a", t0));
    assert!(st.process("b", "b", t0));
    // 同一 key 在限速窗口内被抑制
    assert!(!st.process("a", "a", t0 + Duration::from_millis(100)));
    assert!(!st.process("b", "b", t0 + Duration::from_millis(100)));
    // 超过限速间隔后可再次输出
    assert!(st.process("a", "a", t0 + Duration::from_secs(1)));
    assert!(st.process("b", "b", t0 + Duration::from_secs(1)));
}

#[test]
fn warn_rate_limit_cleanup() {
    let mut st = RateLimitState::default();
    let t0 = Instant::now();

    // 首次调用: 插入 a, 并触发第一次清理 (记录 last_cleanup = t0)
    assert!(st.process("a", "a", t0));
    assert_eq!(st.keys.len(), 1);

    // 1 秒后插入 b: 距上次清理 1 秒, 清理动作本身被限速, 不触发清理
    assert!(st.process("b", "b", t0 + Duration::from_secs(1)));
    assert_eq!(st.keys.len(), 2);

    // 3 秒后调用 a: 距上次清理 3 秒, 超过 2 倍限速间隔, 触发清理.
    // a 本次输出更新了自己的时间, 被保留; b 超过 1 倍间隔未输出, 被丢弃
    assert!(st.process("a", "a", t0 + Duration::from_secs(3)));
    assert_eq!(st.keys.len(), 1);
    assert!(st.keys.contains_key("a"));
    assert!(!st.keys.contains_key("b"));
}
