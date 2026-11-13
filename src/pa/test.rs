//! pa 模块测试.

use std::path::{Path, PathBuf};

use super::{Instance, IpcAcl, PaError, SysdClient, SysdServer, choose_transport, resolve_path};

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
fn resolve_path_relative() {
    let base = Path::new("/a/b");
    assert_eq!(resolve_path(base, "c.db"), PathBuf::from("/a/b/c.db"));
    assert_eq!(resolve_path(base, "/c.db"), PathBuf::from("/c.db"));
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
    let mut server = SysdServer::bind(&name, IpcAcl::CurrentUser).unwrap();
    let client = SysdClient::connect_transport(&name);
    let (conn, client) = tokio::join!(server.accept(), client);
    let mut conn = conn.unwrap();
    let mut client = client.unwrap();

    client.send("hi".to_string()).await.unwrap();
    assert_eq!(conn.recv().await.unwrap(), "hi");

    conn.send("yo".to_string()).await.unwrap();
    assert_eq!(client.recv().await.unwrap(), "yo");
}
