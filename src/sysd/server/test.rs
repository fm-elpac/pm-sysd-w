//! server 模块测试

use std::time::Duration;

use crate::{
    jsonl::{
        Hello, SdReq, SdReqUnit, SdReqUnitOp, SdRes, VERSION, decode_frame, decode_hello,
        encode_frame, encode_hello,
    },
    pa::{Instance, IpcAcl, SysdClient, SysdListener},
    sysd::test_util,
};

use super::{Conn, handle_conn};
use crate::conf::limit::{
    SYSD_LIMIT_IPC_HANDSHAKE_TIMEOUT_MS, SYSD_LIMIT_IPC_IDLE_TIMEOUT_MS, SysdClientLimit, SysdLimit,
};

#[tokio::test]
async fn readonly_connection_rejects_modifying_command() {
    // handle_conn 需要一个 SysD 句柄 (只读拒绝路径不会真正转发命令).
    let (sysd, _logd, _dir) = test_util::make_sysd("pm_tc = 1\n");
    let (sysd_tx, sysd_rx) = tokio::sync::watch::channel(Some(sysd));
    let (_exiting_tx, exiting_rx) = tokio::sync::watch::channel(false);
    let st = Conn::new(sysd_rx, exiting_rx);

    let instance = Instance::Machine(format!("init_ro_{}", std::process::id()));
    let mut server = SysdListener::bind(
        &instance.transport(),
        IpcAcl::CurrentUser,
        SysdLimit::default().ipc(),
    )
    .unwrap();

    let conn = tokio::spawn(async move {
        let conn = server.accept().await.unwrap();
        handle_conn(conn, st, true, 1, SysdLimit::default()).await;
    });

    let mut client = SysdClient::connect(&instance, &SysdClientLimit::default())
        .await
        .unwrap();
    client
        .send(encode_hello(&Hello::new()).unwrap())
        .await
        .unwrap();
    let resp = client.recv().await.unwrap();
    assert_eq!(decode_hello(&resp).unwrap().data.version, VERSION);

    client
        .send(
            encode_frame(&SdReq::Unit(SdReqUnit {
                name: "t.service".to_string(),
                op: SdReqUnitOp::Start,
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let resp = client.recv().await.unwrap();
    let res: SdRes = decode_frame(&resp).unwrap();
    assert!(matches!(&res, SdRes::Err(m) if m.contains("read-only")));

    // 只读连接尝试修改命令后, 服务器关闭连接.
    assert!(matches!(client.recv().await, Err(_)));

    conn.await.unwrap();

    // 只读连接拒绝路径不会真正使用 SysD, 但需要保持句柄存活到测试结束.
    drop(sysd_tx);
}

#[tokio::test(start_paused = true)]
async fn handshake_timeout_closes_connection() {
    let (sysd_tx, sysd_rx) = tokio::sync::watch::channel(None);
    let (_exiting_tx, exiting_rx) = tokio::sync::watch::channel(false);
    let st = Conn::new(sysd_rx, exiting_rx);

    let instance = Instance::Machine(format!("init_handshake_timeout_{}", std::process::id()));
    let mut server = SysdListener::bind(
        &instance.transport(),
        IpcAcl::CurrentUser,
        SysdLimit::default().ipc(),
    )
    .unwrap();

    let conn = tokio::spawn(async move {
        let conn = server.accept().await.unwrap();
        handle_conn(conn, st, false, 1, SysdLimit::default()).await;
    });

    let mut client = SysdClient::connect(&instance, &SysdClientLimit::default())
        .await
        .unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(SYSD_LIMIT_IPC_HANDSHAKE_TIMEOUT_MS + Duration::from_millis(10)).await;

    assert!(matches!(client.recv().await, Err(_)));

    conn.await.unwrap();

    drop(sysd_tx);
}

#[tokio::test(start_paused = true)]
async fn idle_timeout_closes_connection_after_handshake() {
    let (sysd_tx, sysd_rx) = tokio::sync::watch::channel(None);
    let (_exiting_tx, exiting_rx) = tokio::sync::watch::channel(false);
    let st = Conn::new(sysd_rx, exiting_rx);

    let instance = Instance::Machine(format!("init_idle_timeout_{}", std::process::id()));
    let mut server = SysdListener::bind(
        &instance.transport(),
        IpcAcl::CurrentUser,
        SysdLimit::default().ipc(),
    )
    .unwrap();

    let conn = tokio::spawn(async move {
        let conn = server.accept().await.unwrap();
        handle_conn(conn, st, false, 1, SysdLimit::default()).await;
    });

    let mut client = SysdClient::connect(&instance, &SysdClientLimit::default())
        .await
        .unwrap();
    client
        .send(encode_hello(&Hello::new()).unwrap())
        .await
        .unwrap();
    let resp = client.recv().await.unwrap();
    assert_eq!(decode_hello(&resp).unwrap().data.version, VERSION);

    tokio::task::yield_now().await;
    tokio::time::advance(SYSD_LIMIT_IPC_IDLE_TIMEOUT_MS + Duration::from_millis(10)).await;

    assert!(matches!(client.recv().await, Err(_)));

    conn.await.unwrap();

    drop(sysd_tx);
}
