//! p_u 模块测试.

use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::test::TempDir;

use super::{Listener, connect};

fn tmp_sock(dir: &TempDir) -> String {
    dir.path().join("a.sock").to_string_lossy().into_owned()
}

#[tokio::test]
async fn bind_unique_and_cleanup() {
    let dir = TempDir::new("pu");
    let path = tmp_sock(&dir);

    let l1 = Listener::bind(&path).unwrap();
    assert!(Path::new(&path).exists());

    // 同名实例 再次 bind 失败 (唯一性检测).
    assert!(Listener::bind(&path).is_err());

    // 释放监听器 后, socket 文件 被自动删除.
    drop(l1);
    assert!(!Path::new(&path).exists());
}

#[tokio::test]
async fn connect_roundtrip() {
    let dir = TempDir::new("pu");
    let path = tmp_sock(&dir);
    let mut listener = Listener::bind(&path).unwrap();

    let connect = connect(&path);
    let (accepted, client) = tokio::join!(listener.accept(), connect);
    let mut server = accepted.unwrap();
    let mut client = client.unwrap();

    server.write_all(b"hi").await.unwrap();
    let mut buf = [0u8; 2];
    client.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hi");
}
