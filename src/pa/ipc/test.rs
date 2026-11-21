//! ipc 模块测试.

use std::time::Duration;

use tokio::io::{AsyncWriteExt, BufReader, duplex};

use super::{Instance, IpcAcl, PaError, SysdClient, SysdListener, choose_transport, recv_line};
use crate::conf::limit::{
    SYSD_LIMIT_IPC_M_BYTE, SYSD_LIMIT_IPC_RECV_TIMEOUT_MS, SysdClientLimit, SysdLimit,
};

#[test]
fn instance_machine_name() {
    let i = Instance::Machine("dev".to_string());
    assert_eq!(i.name(), "pm_sysd_w_m_dev");
}

#[test]
fn instance_system_name() {
    assert_eq!(Instance::System.name(), "pm_sysd_w_system_default");
}

#[test]
fn instance_user_name_prefix() {
    assert!(Instance::User.name().starts_with("pm_sysd_w_user_"));
}

#[test]
fn instance_transport() {
    let i = Instance::Machine("dev".to_string());
    #[cfg(target_os = "linux")]
    assert_eq!(i.transport(), "./pm_sysd_w_m_dev.sock");
    #[cfg(windows)]
    assert_eq!(i.transport(), "pm_sysd_w_m_dev");
}

#[test]
fn choose_transport_not_found() {
    let instance = Instance::Machine("nope".to_string());
    let r = choose_transport(&instance, |_| false, None);
    assert!(matches!(r, Err(PaError::IpcNotFound(_))));
}

#[test]
fn choose_transport_success() {
    let instance = Instance::Machine("dev".to_string());
    let transport = instance.transport();
    let r = choose_transport(&instance, |p| p == transport.as_str(), None);
    assert_eq!(r.unwrap(), transport);
}

#[cfg(windows)]
#[test]
fn choose_transport_system_permission_fallback() {
    let instance = Instance::System;
    let main = instance.transport();
    let ro = instance.ro_transport().unwrap();

    let r = choose_transport(&instance, |p| p == main.as_str() || p == ro.as_str(), None);
    assert_eq!(r.unwrap(), main);

    let r = choose_transport(
        &instance,
        |p| p == main.as_str() || p == ro.as_str(),
        Some(PaError::IpcPermission("denied".into())),
    );
    assert_eq!(r.unwrap(), ro);
}

#[tokio::test]
async fn sysd_server_client_roundtrip() {
    let name = format!("pm_sysd_w_pa_test_{}", std::process::id());
    let mut server =
        SysdListener::bind(&name, IpcAcl::CurrentUser, SysdLimit::default().ipc()).unwrap();
    let client_limit = SysdClientLimit::default();
    let client = SysdClient::connect_transport(&name, &client_limit);
    let (conn, client) = tokio::join!(server.accept(), client);
    let mut conn = conn.unwrap();
    let mut client = client.unwrap();

    client.send("hi".to_string()).await.unwrap();
    assert_eq!(conn.recv().await.unwrap(), "hi");

    conn.send("yo".to_string()).await.unwrap();
    assert_eq!(client.recv().await.unwrap(), "yo");
}

/// 对方完全没发消息时, `recv_line` 不应因 SYSD_LIMIT_IPC_RECV_TIMEOUT_MS 超时, 而是一直等待.
#[tokio::test(start_paused = true)]
async fn recv_line_waits_without_message() {
    let (local, _remote) = duplex(64);
    let mut reader = BufReader::new(local);
    let handle = tokio::spawn(async move {
        recv_line(
            &mut reader,
            SYSD_LIMIT_IPC_RECV_TIMEOUT_MS,
            SYSD_LIMIT_IPC_M_BYTE,
        )
        .await
    });

    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(10)).await;

    assert!(!handle.is_finished());
    handle.abort();
}

/// 收到消息第一个字节后, 剩余部分若迟迟不到, 应在 SYSD_LIMIT_IPC_RECV_TIMEOUT_MS 内超时.
#[tokio::test(start_paused = true)]
async fn recv_line_times_out_after_first_byte() {
    let (mut local, remote) = duplex(64);
    local.write_all(b"partial").await.unwrap();
    let mut reader = BufReader::new(remote);

    let handle = tokio::spawn(async move {
        recv_line(
            &mut reader,
            SYSD_LIMIT_IPC_RECV_TIMEOUT_MS,
            SYSD_LIMIT_IPC_M_BYTE,
        )
        .await
    });

    tokio::task::yield_now().await;
    tokio::time::advance(SYSD_LIMIT_IPC_RECV_TIMEOUT_MS + Duration::from_millis(10)).await;

    let r = handle.await.unwrap();
    assert!(matches!(r, Err(PaError::Timeout(_))));
}
