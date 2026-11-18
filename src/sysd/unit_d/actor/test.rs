//! UnitD actor 集成测试 (unit 管理: 配置, enable/mask, daemon-reload, default target).

use tokio::sync::oneshot;

use crate::{
    conf::UnitType,
    db::{DbClient, UnitState},
    jsonl::{SdReq, SdReqUnitOp, SdReqUnitd, SdReqUnitdRO, SdRes},
    sysd::{SysDM, test_util::*},
    sysu::{EnableSta, LoadSta, UnitSta},
};

#[tokio::test]
async fn unknown_unit() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    assert!(svc_start(&sysd, "nope.service").await.is_err());
    assert!(svc_stop(&sysd, "nope.service").await.is_err());
    assert!(status(&sysd, "nope.service").await.is_err());
}

#[tokio::test]
async fn list_units_sorted() {
    let text = multi_units(&[
        ("b.service", "default.target"),
        ("a.service", "default.target"),
    ]);
    let (sysd, _logd, _dir) = make_sysd(&text);

    let units = list_units(&sysd).await;
    assert_eq!(units.len(), 3);
    assert_eq!(units[0].name, "a.service");
    assert_eq!(units[1].name, "b.service");
    assert_eq!(units[2].name, "default.target");
    assert_eq!(units[2].unit_type, UnitType::Target);
}

#[tokio::test]
async fn unit_config() {
    let text = unit_conf(
        "bin --x",
        r#""env:Service.Restart" = "always"
"env:Service.RestartSec" = "3"
"env:Service.ExecStop" = "stop"
"env:Service.ExecReload" = "reload"
"env:Install.WantedBy" = "default.target"
"#,
    );
    let (sysd, _logd, _dir) = make_sysd(&text);

    let c = config(&sysd, "t.service").await.unwrap();
    assert_eq!(c.name, "t.service");
    assert_eq!(
        c.service.exec_start,
        vec![vec!["bin".to_string(), "--x".to_string()]]
    );
    assert_eq!(c.service.restart.as_str(), "always");
    assert_eq!(c.service.restart_sec, 3);
    assert_eq!(c.service.exec_stop, vec![vec!["stop".to_string()]]);
    assert_eq!(c.service.exec_reload, vec![vec!["reload".to_string()]]);
    assert_eq!(c.install.wanted_by, vec!["default.target".to_string()]);
}

#[tokio::test]
async fn list_dependencies_empty() {
    let text = unit_conf(&ok_cmd(), r#""env:Install.WantedBy" = "default.target""#);
    let (sysd, _logd, _dir) = make_sysd(&text);
    assert!(
        list_dependencies(&sysd, "t.service")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn enable_disable_is_enabled() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(
        &run_cmd(),
        r#""env:Install.WantedBy" = "default.target""#,
    ));

    assert!(!unit_enabled(&sysd, "t.service").await);

    unit_enable(&sysd, "t.service", false).await.unwrap();
    assert!(unit_enabled(&sysd, "t.service").await);

    unit_mask(&sysd, "t.service", false).await.unwrap();
    assert!(unit_masked(&sysd, "t.service").await);

    unit_unmask(&sysd, "t.service").await.unwrap();
    unit_disable(&sysd, "t.service", false).await.unwrap();
    assert!(!unit_enabled(&sysd, "t.service").await);
}

#[tokio::test]
async fn enable_now_starts_and_disable_now_stops() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(
        &run_cmd(),
        r#""env:Install.WantedBy" = "default.target""#,
    ));

    unit_enable(&sysd, "t.service", true).await.unwrap();
    assert!(is_active(&sysd, "t.service").await);

    unit_disable(&sysd, "t.service", true).await.unwrap();
    assert!(!is_active(&sysd, "t.service").await);
    assert!(!unit_enabled(&sysd, "t.service").await);
}

#[tokio::test]
async fn reenable() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(
        &run_cmd(),
        r#""env:Install.WantedBy" = "default.target""#,
    ));
    unit_enable(&sysd, "t.service", false).await.unwrap();
    unit_disable(&sysd, "t.service", false).await.unwrap();
    unit_reenable(&sysd, "t.service").await.unwrap();
    assert!(unit_enabled(&sysd, "t.service").await);
}

#[tokio::test]
async fn preset_disables_enableable_unit() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(
        &run_cmd(),
        r#""env:Install.WantedBy" = "default.target""#,
    ));

    unit_enable(&sysd, "t.service", false).await.unwrap();
    assert!(unit_enabled(&sysd, "t.service").await);

    unit_preset(&sysd, "t.service").await.unwrap();
    assert!(!unit_enabled(&sysd, "t.service").await);
}

#[tokio::test]
async fn preset_ignores_static_unit() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    // 无 [Install] 配置的 unit 不支持 enable/disable, preset 应成功忽略.
    unit_preset(&sysd, "t.service").await.unwrap();

    let s = unit_state(&sysd, "t.service").await.unwrap();
    assert_eq!(s.enable, EnableSta::Static);
}

#[tokio::test]
async fn preset_missing_unit_fails() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    assert!(unit_preset(&sysd, "nope.service").await.is_err());
}

#[tokio::test]
async fn mask_now_stops_and_blocks_start() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();
    unit_mask(&sysd, "t.service", true).await.unwrap();
    assert!(!is_active(&sysd, "t.service").await);
    assert!(unit_masked(&sysd, "t.service").await);
    assert!(svc_start(&sysd, "t.service").await.is_err());

    unit_unmask(&sysd, "t.service").await.unwrap();
    svc_start(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn mask_missing_unit_succeeds_and_reports_masked() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    // 配置中不存在 nope.service, 但 mask 仍然成功.
    unit_mask(&sysd, "nope.service", false).await.unwrap();

    // status 返回 被 mask 的 unit 信息, 而不是 not found.
    let st = status(&sysd, "nope.service").await.unwrap();
    assert_eq!(st.name, "nope.service");
    assert_eq!(st.load, LoadSta::Masked);
    assert_eq!(st.enable, EnableSta::Masked);
    assert_eq!(st.active, UnitSta::Inactive);

    // list-units 不显示 被 mask 的 unit.
    let units = list_units(&sysd).await;
    assert!(!units.iter().any(|u| u.name == "nope.service"));

    // unmask 后 又回到 不存在.
    unit_unmask(&sysd, "nope.service").await.unwrap();
    assert!(status(&sysd, "nope.service").await.is_err());
}

#[tokio::test]
async fn mask_missing_unit_returns_warning() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    let resp = handle(
        &sysd,
        &SdReq::Unit(unit("nope.service", SdReqUnitOp::Mask { now: false })),
    )
    .await;
    match resp {
        SdRes::Ok(Some(msg)) => {
            assert_eq!(msg, "Unit nope.service does not exist, proceeding anyway.")
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[tokio::test]
async fn mask_without_now_keeps_running_service() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();
    unit_mask(&sysd, "t.service", false).await.unwrap();

    // 不加 --now: service 仍在运行, 但查询显示 masked, 其余信息来自旧配置.
    assert!(unit_masked(&sysd, "t.service").await);

    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(st.load, LoadSta::Masked);
    assert_eq!(st.enable, EnableSta::Masked);
    assert_eq!(st.active, UnitSta::Active);
    assert_eq!(st.description, "test service");

    let s = unit_state(&sysd, "t.service").await.unwrap();
    assert_eq!(s.load, LoadSta::Masked);
    assert_eq!(s.enable, EnableSta::Masked);
    assert_eq!(s.active, UnitSta::Active);
    assert_eq!(s.description, "test service");

    // 被 mask 但仍在运行的 service 也要出现在 list-units 中, 并显示 masked.
    let units = list_units(&sysd).await;
    let listed = units.iter().find(|u| u.name == "t.service").unwrap();
    assert_eq!(listed.load, LoadSta::Masked);
    assert_eq!(listed.enable, EnableSta::Masked);
    assert_eq!(listed.active, UnitSta::Active);

    // start 仍被阻止.
    assert!(svc_start(&sysd, "t.service").await.is_err());

    // unmask 后 service 仍可继续运行.
    unit_unmask(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn masked_unit_is_hidden_and_status_only_shows_mask() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    unit_mask(&sysd, "t.service", false).await.unwrap();

    // 被 mask 的 unit 不出现在 list-units 中.
    let units = list_units(&sysd).await;
    assert!(!units.iter().any(|u| u.name == "t.service"));

    // 不显示 配置详情.
    assert!(config(&sysd, "t.service").await.is_err());
    assert!(list_dependencies(&sysd, "t.service").await.is_err());

    // status 只显示 mask, 不显示具体详情.
    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(st.load, LoadSta::Masked);
    assert_eq!(st.enable, EnableSta::Masked);
    assert_eq!(st.active, UnitSta::Inactive);
    assert_eq!(st.description, "");

    // is-enabled 显示 masked.
    let s = unit_state(&sysd, "t.service").await.unwrap();
    assert_eq!(s.enable, EnableSta::Masked);
}

#[tokio::test]
async fn masked_unit_blocks_start_like_ops() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    unit_mask(&sysd, "t.service", false).await.unwrap();

    assert!(svc_start(&sysd, "t.service").await.is_err());
    assert!(svc_restart(&sysd, "t.service").await.is_err());
    assert!(svc_reload_or_restart(&sysd, "t.service").await.is_err());
    assert!(svc_try_restart(&sysd, "t.service").await.is_err());
    assert!(svc_try_reload_or_restart(&sysd, "t.service").await.is_err());
}

#[tokio::test]
async fn daemon_reload_keeps_masked_missing_unit() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    unit_mask(&sysd, "nope.service", false).await.unwrap();

    daemon_reload(&sysd).await.unwrap();

    let st = status(&sysd, "nope.service").await.unwrap();
    assert_eq!(st.load, LoadSta::Masked);
}

#[tokio::test]
async fn get_default_set_default() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    assert_eq!(get_default(&sysd).await, "default.target");

    set_default(&sysd, "default.target").await.unwrap();
    assert_eq!(get_default(&sysd).await, "default.target");
    assert!(set_default(&sysd, "foo.target").await.is_err());
    assert_eq!(get_default(&sysd).await, "default.target");
}

#[tokio::test]
async fn daemon_reload_reloads_units() {
    let (sysd, _logd, dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    assert_eq!(list_units(&sysd).await.len(), 2);

    let new_text = multi_units(&[
        ("a.service", "default.target"),
        ("b.service", "default.target"),
    ]);
    std::fs::write(dir.path().join("c.toml"), new_text).unwrap();
    daemon_reload(&sysd).await.unwrap();

    let units = list_units(&sysd).await;
    assert_eq!(units.len(), 3);
    assert_eq!(units[0].name, "a.service");
}

#[tokio::test]
async fn start_default_target() {
    let text = multi_units(&[
        ("a.service", "default.target"),
        ("b.service", "timers.target"),
        ("c.service", "default.target"),
        ("d.service", "default.target"),
    ]);
    let (sysd, _logd, _dir) = make_sysd(&text);

    unit_enable(&sysd, "a.service", false).await.unwrap();
    assert!(unit_enable(&sysd, "b.service", false).await.is_err());
    unit_enable(&sysd, "d.service", false).await.unwrap();
    unit_mask(&sysd, "d.service", false).await.unwrap();

    let (tx, rx) = oneshot::channel();
    sysd.send(SysDM::StartDefaultTarget(tx)).await.unwrap();
    rx.await.unwrap().unwrap();

    assert!(is_active(&sysd, "a.service").await);
    assert!(!is_active(&sysd, "b.service").await);
    assert!(!is_active(&sysd, "c.service").await);
    assert!(!is_active(&sysd, "d.service").await);
}

#[tokio::test]
async fn handle_todo_commands() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    let cmds = vec![
        SdReq::Unit(unit("t.service", SdReqUnitOp::Clean)),
        SdReq::UnitdRO(SdReqUnitdRO::ShowEnvironment),
        SdReq::UnitD(SdReqUnitd::SetEnvironment),
        SdReq::UnitD(SdReqUnitd::UnsetEnvironment),
    ];
    for cmd in cmds {
        match handle(&sysd, &cmd).await {
            SdRes::Err(message) => assert!(message.contains("TODO")),
            other => panic!("unexpected response: {:?}", other),
        }
    }
}

#[tokio::test]
async fn daemon_reload_keeps_running_service() {
    let (sysd, _logd, dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();

    // 重新加载 一个 移除 t.service 的 配置; 运行中的 service 仍应保留旧配置.
    std::fs::write(dir.path().join("c.toml"), "pm_tc = 1\n").unwrap();
    daemon_reload(&sysd).await.unwrap();

    assert!(is_active(&sysd, "t.service").await);
    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(st.name, "t.service");

    // list-units 应显示 已从新配置移除但仍运行的 unit.
    let units = list_units(&sysd).await;
    assert!(units.iter().any(|u| u.name == "t.service"));

    // 停止后 不再 active (已从配置移除, status 不再可查).
    svc_stop(&sysd, "t.service").await.unwrap();
    assert!(status(&sysd, "t.service").await.is_err());
}

#[tokio::test]
async fn daemon_reload_reads_mask_enable_from_db() {
    let (sysd, _logd, dir) = make_sysd(&unit_conf(
        &run_cmd(),
        r#""env:Install.WantedBy" = "default.target""#,
    ));

    // 先等待 UnitD 启动并完成初始 mask/enable 加载 (任何 ctl 响应都意味着已就绪).
    get_default(&sysd).await;

    // 直接通过另一个数据库客户端修改 enable, 绕过 UnitD 内存状态.
    let db_path = dir.path().join("pm_sysd_w_v1.db");
    let db_path = db_path.to_string_lossy().into_owned();
    let db = DbClient::open(&db_path).unwrap();
    db.unit_write(&UnitState {
        name: "t.service".into(),
        enabled: true,
        masked: false,
    })
    .await
    .unwrap();

    // reload 前 内存中仍是 disabled.
    assert!(!unit_enabled(&sysd, "t.service").await);

    daemon_reload(&sysd).await.unwrap();

    // reload 后 从数据库重新读取到 enabled.
    assert!(unit_enabled(&sysd, "t.service").await);
}
