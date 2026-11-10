//! jsonl 模块测试.

use super::*;

#[test]
fn hello_new() {
    let h = Hello::new();
    assert_eq!(h.protocol, PROTOCOL_VERSION);
    assert_eq!(h.data.version, VERSION);
}

#[test]
fn hello_roundtrip() {
    let h = Hello::new();
    let line = encode_hello(&h).unwrap();
    let h2 = decode_hello(&line).unwrap();
    assert_eq!(h2.protocol, h.protocol);
    assert_eq!(h2.data.version, h.data.version);
    check_version(&h2).unwrap();
}

#[test]
fn hello_json_shape() {
    let line = encode_hello(&Hello::new()).unwrap();
    assert!(line.contains("\"pm-sysd-w\":1"));
    assert!(line.contains("\"data\":{"));
}

#[test]
fn check_version_protocol_mismatch() {
    let mut h = Hello::new();
    h.protocol = 2;
    assert!(check_version(&h).is_err());
}

#[test]
fn check_version_software_mismatch() {
    let mut h = Hello::new();
    h.data.version = "0.0.0".into();
    assert!(check_version(&h).is_err());
}

#[test]
fn decode_invalid() {
    assert!(decode_hello("not json").is_err());
    assert!(decode_frame("not json").is_err());
    assert!(decode_frame("{}").is_err());
}

#[test]
fn command_roundtrip() {
    let cases = vec![
        Command::ListUnits,
        Command::ListTimers,
        Command::Logs {
            query: LogQuery {
                unit: Some("a.service".into()),
                limit: 100,
            },
        },
        Command::IsActive {
            unit: "a.service".into(),
        },
        Command::IsFailed {
            unit: "a.service".into(),
        },
        Command::Status {
            unit: "a.service".into(),
        },
        Command::Show {
            unit: "a.service".into(),
        },
        Command::Cat {
            unit: "a.service".into(),
        },
        Command::ListDependencies {
            unit: "a.service".into(),
        },
        Command::Start {
            unit: "a.service".into(),
        },
        Command::Stop {
            unit: "a.service".into(),
        },
        Command::Reload {
            unit: "a.service".into(),
        },
        Command::Restart {
            unit: "a.service".into(),
        },
        Command::TryRestart {
            unit: "a.service".into(),
        },
        Command::ReloadOrRestart {
            unit: "a.service".into(),
        },
        Command::TryReloadOrRestart {
            unit: "a.service".into(),
        },
        Command::Kill {
            unit: "a.service".into(),
        },
        Command::Clean {
            unit: "a.service".into(),
        },
        Command::ResetFailed {
            unit: Some("a.service".into()),
        },
        Command::Enable {
            unit: "a.service".into(),
            now: true,
        },
        Command::Disable {
            unit: "a.service".into(),
            now: false,
        },
        Command::Reenable {
            unit: "a.service".into(),
        },
        Command::Preset {
            unit: "a.service".into(),
        },
        Command::IsEnabled {
            unit: "a.service".into(),
        },
        Command::Mask {
            unit: "a.service".into(),
        },
        Command::Unmask {
            unit: "a.service".into(),
        },
        Command::GetDefault,
        Command::SetDefault {
            target: "default.target".into(),
        },
        Command::ShowEnvironment,
        Command::SetEnvironment,
        Command::UnsetEnvironment,
        Command::ImportEnvironment,
        Command::DaemonReload,
        Command::SoftReboot,
        Command::Exit,
    ];

    for cmd in cases {
        let line = encode_frame(&Frame::Command(cmd.clone())).unwrap();
        let f = decode_frame(&line).unwrap();
        match f {
            Frame::Command(c2) => assert_eq!(format!("{:?}", cmd), format!("{:?}", c2)),
            other => panic!("expected Command, got {:?}", other),
        }
    }
}

#[test]
fn response_roundtrip() {
    let cases = vec![
        Response::Ok,
        Response::Err {
            message: "boom".into(),
        },
        Response::Bool { value: true },
        Response::ListUnits {
            units: vec![UnitInfo {
                name: "a.service".into(),
                description: "d".into(),
                active: ActiveState::Active,
                sub: SubState::Running,
            }],
        },
        Response::Status {
            status: UnitStatus {
                name: "a.service".into(),
                description: "d".into(),
                enabled: true,
                masked: false,
                active: ActiveState::Active,
                sub: SubState::Running,
                pid: Some(42),
                restart_count: 0,
                exit_status: None,
            },
        },
        Response::Config {
            config: UnitConfig {
                name: "a.service".into(),
                description: "d".into(),
                service_type: "simple".into(),
                exec_start: "bin".into(),
                exec_stop: None,
                exec_reload: None,
                restart: "no".into(),
                restart_sec: 1,
                wanted_by: None,
            },
        },
        Response::EnableState {
            state: EnableState::Enabled,
        },
        Response::DefaultTarget {
            target: "default.target".into(),
        },
        Response::Dependencies {
            deps: vec!["default.target".into()],
        },
        Response::Timers {
            timers: vec![TimerInfo {
                name: "t.timer".into(),
            }],
        },
        Response::Logs {
            logs: vec![LogEntry {
                id: 1,
                ts: "2026-11-10T00:00:00.000Z".into(),
                level: "info".into(),
                unit: Some("a.service".into()),
                msg: "m".into(),
            }],
        },
    ];

    for resp in cases {
        let line = encode_frame(&Frame::Response(resp.clone())).unwrap();
        let f = decode_frame(&line).unwrap();
        match f {
            Frame::Response(r2) => assert_eq!(format!("{:?}", resp), format!("{:?}", r2)),
            other => panic!("expected Response, got {:?}", other),
        }
    }
}

#[test]
fn command_json_shape() {
    let line = encode_frame(&Frame::Command(Command::Start {
        unit: "a.service".into(),
    }))
    .unwrap();
    assert!(line.contains("\"command\":\"start\""));
    assert!(line.contains("\"unit\":\"a.service\""));
}

#[tokio::test]
async fn hello_io_roundtrip() {
    let (mut a, mut b) = tokio::io::duplex(1024);

    send_hello(&mut a, &Hello::new()).await.unwrap();
    let h = recv_hello(&mut b).await.unwrap();
    assert_eq!(h.protocol, PROTOCOL_VERSION);
    assert_eq!(h.data.version, VERSION);
}

#[tokio::test]
async fn frame_io_roundtrip() {
    let (mut a, mut b) = tokio::io::duplex(1024);

    let cmd = Command::Start {
        unit: "a.service".into(),
    };
    send_frame(&mut a, &Frame::Command(cmd.clone()))
        .await
        .unwrap();
    let f = recv_frame(&mut b).await.unwrap();
    match f {
        Frame::Command(c) => assert_eq!(format!("{:?}", cmd), format!("{:?}", c)),
        other => panic!("expected Command, got {:?}", other),
    }
}

#[tokio::test]
async fn recv_hello_eof() {
    let (mut a, b) = tokio::io::duplex(1024);
    drop(b);
    assert!(matches!(recv_hello(&mut a).await, Err(JsonlError::Eof)));
}
