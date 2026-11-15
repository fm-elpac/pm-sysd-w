//! init 模块测试

use crate::{
    jsonl::{
        Hello, SdReq, SdReqLogQuery, SdReqLogRO, SdReqSysd, SdReqUnit, SdReqUnitOp, SdReqUnitOpRO,
        SdReqUnitRO, SdReqUnitd, SdReqUnitdRO, SdRes, VERSION, decode_frame, decode_hello,
        encode_frame, encode_hello,
    },
    pa::{Instance, IpcAcl, SysdClient, SysdServer, ipc_exists},
    sysd::test_util,
    test::TempDir,
};

use super::{Shutdown, handle_conn};

#[test]
fn is_readonly_accepts_read_queries() {
    assert!(SdReq::is_readonly(&SdReq::LogRO(SdReqLogRO::Query(
        SdReqLogQuery {
            level: None,
            unit: None,
            limit: 0,
            cursor: None,
        }
    ))));
    assert!(SdReq::is_readonly(&SdReq::UnitdRO(SdReqUnitdRO::ListUnits)));
    assert!(SdReq::is_readonly(&SdReq::UnitRO(SdReqUnitRO {
        name: "t.service".to_string(),
        op: SdReqUnitOpRO::Status,
    })));
}

#[test]
fn is_readonly_rejects_modifying_commands() {
    assert!(!SdReq::is_readonly(&SdReq::SysD(SdReqSysd::Exit)));
    assert!(!SdReq::is_readonly(&SdReq::UnitD(SdReqUnitd::DaemonReload)));
    assert!(!SdReq::is_readonly(&SdReq::Unit(SdReqUnit {
        name: "t.service".to_string(),
        op: SdReqUnitOp::Start,
    })));
}

#[tokio::test]
async fn readonly_connection_rejects_modifying_command() {
    // handle_conn 需要一个 SysD 句柄 (只读拒绝路径不会真正转发命令).
    let (sysd, _logd, _dir) = test_util::make_sysd("pm_tc = 1\n");

    let instance = Instance::Machine(format!("init_ro_{}", std::process::id()));
    let mut server = SysdServer::bind(&instance.transport(), IpcAcl::CurrentUser).unwrap();

    let conn = tokio::spawn(async move {
        let conn = server.accept().await.unwrap();
        handle_conn(conn, sysd, true).await;
    });

    let mut client = SysdClient::connect(&instance).await.unwrap();
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
}

/// 等待实例 IPC 端点出现 并 完成握手, 返回 客户端连接.
async fn connect_client(instance: &Instance) -> SysdClient {
    let transport = instance.transport();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ipc_exists(&transport) {
        assert!(std::time::Instant::now() < deadline, "IPC not bound");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    let mut client = SysdClient::connect(instance).await.unwrap();
    client
        .send(encode_hello(&Hello::new()).unwrap())
        .await
        .unwrap();
    let resp = client.recv().await.unwrap();
    assert_eq!(decode_hello(&resp).unwrap().data.version, VERSION);
    client
}

#[tokio::test]
async fn run_exit_via_client() {
    let dir = TempDir::new("init");
    let conf_path = dir.path().join("c.toml");
    std::fs::write(&conf_path, "pm_tc = 1\n").unwrap();
    let conf_path = conf_path.to_string_lossy().into_owned();

    let instance = Instance::Machine(format!("init_exit_{}", std::process::id()));
    let task = tokio::spawn(super::run(instance.clone(), conf_path));

    let mut client = connect_client(&instance).await;
    client
        .send(encode_frame(&SdReq::SysD(SdReqSysd::Exit)).unwrap())
        .await
        .unwrap();
    let resp = client.recv().await.unwrap();
    let res: SdRes = decode_frame(&resp).unwrap();
    assert!(matches!(res, SdRes::Ok(_)));

    let shutdown = task.await.unwrap().unwrap();
    assert_eq!(shutdown, Shutdown::Exit);
}

#[tokio::test]
async fn run_soft_reboot_via_client() {
    let dir = TempDir::new("init");
    let conf_path = dir.path().join("c.toml");
    std::fs::write(&conf_path, "pm_tc = 1\n").unwrap();
    let conf_path = conf_path.to_string_lossy().into_owned();

    let instance = Instance::Machine(format!("init_reboot_{}", std::process::id()));
    let task = tokio::spawn(super::run(instance.clone(), conf_path));

    let mut client = connect_client(&instance).await;
    client
        .send(encode_frame(&SdReq::SysD(SdReqSysd::SoftReboot)).unwrap())
        .await
        .unwrap();
    let resp = client.recv().await.unwrap();
    let res: SdRes = decode_frame(&resp).unwrap();
    assert!(matches!(res, SdRes::Ok(_)));

    let shutdown = task.await.unwrap().unwrap();
    assert_eq!(shutdown, Shutdown::SoftReboot);
}

#[tokio::test]
async fn run_bad_conf_returns_error() {
    let dir = TempDir::new("init");
    let conf_path = dir
        .path()
        .join("missing.toml")
        .to_string_lossy()
        .into_owned();

    let instance = Instance::Machine(format!("init_bad_{}", std::process::id()));
    let r = super::run(instance, conf_path).await;
    assert!(r.is_err());
}
