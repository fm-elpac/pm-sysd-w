//! jsonl 模块测试.

use crate::conf::{Install, RestartPolicy, Service, ServiceType, Unit, UnitType, Win};

use super::{
    Hello, PROTOCOL_VERSION, SdReq, SdReqLogQuery, SdReqLogRO, SdReqSysd, SdReqUnit, SdReqUnitOp,
    SdReqUnitOpRO, SdReqUnitRO, SdReqUnitd, SdReqUnitdRO, SdRes, SdResLogEntry, SdResServiceStatus,
    SdResTimerInfo, SdResUnitS, SdResUnitState, SdResUnitStatus, VERSION, check_version,
    decode_frame, decode_hello, encode_frame, encode_hello,
};

use crate::sysd::{ServiceSta, UnitSta, UnitSubSta};

/// 构造一个 最小可用的 service unit 配置 (供响应 roundtrip 使用).
fn test_unit() -> Unit {
    Unit {
        name: "a.service".into(),
        unit_type: UnitType::Service,
        description: "d".into(),
        start_limit_interval_sec: 10,
        start_limit_burst: 5,
        service: Service {
            service_type: ServiceType::Simple,
            exec_start: vec!["bin".to_string()],
            exec_stop: None,
            exec_reload: None,
            restart: RestartPolicy::No,
            restart_sec: 1,
            timeout_start_sec: 90,
            timeout_stop_sec: 90,
            working_directory: None,
            environment: Vec::new(),
            remain_after_exit: false,
        },
        install: Install { wanted_by: None },
        win: Win {
            create_no_window: false,
        },
    }
}

/// 构造 针对 单个 unit 的 修改 请求.
fn unit(name: &str, op: SdReqUnitOp) -> SdReqUnit {
    SdReqUnit {
        name: name.to_string(),
        op,
    }
}

/// 构造 针对 单个 unit 的 只读查询 请求.
fn unit_ro(name: &str, op: SdReqUnitOpRO) -> SdReqUnitRO {
    SdReqUnitRO {
        name: name.to_string(),
        op,
    }
}

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
    assert!(decode_frame::<SdReq>("not json").is_err());
    assert!(decode_frame::<SdReq>("{}").is_err());
}

#[test]
fn command_roundtrip() {
    let cases = vec![
        SdReq::UnitdRO(SdReqUnitdRO::ListUnits),
        SdReq::UnitdRO(SdReqUnitdRO::ListTimers),
        SdReq::LogRO(SdReqLogRO::Query(SdReqLogQuery {
            unit: Some("a.service".into()),
            limit: 100,
            cursor: None,
        })),
        SdReq::UnitRO(unit_ro("a.service", SdReqUnitOpRO::UnitState)),
        SdReq::UnitRO(unit_ro("a.service", SdReqUnitOpRO::Status)),
        SdReq::UnitRO(unit_ro("a.service", SdReqUnitOpRO::Config)),
        SdReq::UnitRO(unit_ro("a.service", SdReqUnitOpRO::ListDependencies)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Start)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Stop)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Reload)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Restart)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::TryRestart)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::ReloadOrRestart)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::TryReloadOrRestart)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Kill)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Clean)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::ResetFailed)),
        SdReq::Unit(unit("", SdReqUnitOp::ResetFailed)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Enable { now: true })),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Disable { now: false })),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Reenable)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Preset)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Mask)),
        SdReq::Unit(unit("a.service", SdReqUnitOp::Unmask)),
        SdReq::UnitdRO(SdReqUnitdRO::GetDefault),
        SdReq::UnitD(SdReqUnitd::SetDefault {
            target: "default.target".into(),
        }),
        SdReq::UnitdRO(SdReqUnitdRO::ShowEnvironment),
        SdReq::UnitD(SdReqUnitd::SetEnvironment),
        SdReq::UnitD(SdReqUnitd::UnsetEnvironment),
        SdReq::UnitD(SdReqUnitd::DaemonReload),
        SdReq::SysD(SdReqSysd::SoftReboot),
        SdReq::SysD(SdReqSysd::Exit),
    ];

    for cmd in cases {
        let line = encode_frame(&cmd).unwrap();
        let c2 = decode_frame::<SdReq>(&line).unwrap();
        assert_eq!(format!("{:?}", cmd), format!("{:?}", c2));
    }
}

#[test]
fn response_roundtrip() {
    let cases = vec![
        SdRes::Ok,
        SdRes::Err("boom".into()),
        SdRes::UnitState(Some(SdResUnitState {
            name: "a.service".into(),
            description: "d".into(),
            unit_type: UnitType::Service,
            active: UnitSta::Active,
            sub: UnitSubSta::Service(ServiceSta::Running),
            enabled: false,
            masked: false,
        })),
        SdRes::ListUnits(vec![SdResUnitState {
            name: "a.service".into(),
            description: "d".into(),
            unit_type: UnitType::Service,
            active: UnitSta::Active,
            sub: UnitSubSta::Service(ServiceSta::Running),
            enabled: false,
            masked: false,
        }]),
        SdRes::Status(SdResUnitStatus {
            name: "a.service".into(),
            description: "d".into(),
            enabled: true,
            masked: false,
            active: UnitSta::Active,
            s: SdResUnitS::Service(SdResServiceStatus {
                sub: ServiceSta::Running,
                pid: Some(42),
                restart_count: 0,
                exit_status: None,
            }),
        }),
        SdRes::Config(test_unit()),
        SdRes::DefaultTarget("default.target".into()),
        SdRes::Dependencies(vec!["default.target".into()]),
        SdRes::Timers(vec![SdResTimerInfo {
            name: "t.timer".into(),
        }]),
        SdRes::Logs {
            total: 1,
            cursor: None,
            logs: vec![SdResLogEntry {
                id: 1,
                ts: "2026-11-10T00:00:00.000Z".into(),
                level: "info".into(),
                unit: Some("a.service".into()),
                msg: "m".into(),
            }],
        },
    ];

    for resp in cases {
        let line = encode_frame(&resp).unwrap();
        let r2 = decode_frame::<SdRes>(&line).unwrap();
        assert_eq!(format!("{:?}", resp), format!("{:?}", r2));
    }
}

#[test]
fn command_json_shape() {
    let line = encode_frame(&SdReq::Unit(unit("a.service", SdReqUnitOp::Start))).unwrap();
    assert!(line.contains("\"req\":\"unit\""));
    assert!(line.contains("\"op\":\"start\""));
    assert!(line.contains("\"name\":\"a.service\""));
}
