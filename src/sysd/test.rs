//! sysd 模块测试 (多 actor 架构, 通过公开句柄 `SysD` 驱动).

use std::{future::Future, pin::Pin};

use tokio::sync::{mpsc, oneshot};

use crate::{
    conf,
    db::DbClient,
    jsonl::{
        SdReq, SdReqSysd, SdReqUnit, SdReqUnitOp, SdReqUnitOpRO, SdReqUnitRO, SdReqUnitd,
        SdReqUnitdRO, SdRes, SdResServiceStatus, SdResUnitS, SdResUnitState, SdResUnitStatus,
    },
    logd::Logd,
    sysd::{ServiceSta, UnitSta, UnitSubSta},
    test::TempDir,
};

use super::init::Shutdown;
use super::{SysD, SysDM, SysdError};

/// 一个 立即成功退出 (退出码 0) 的 命令 (跨平台).
fn ok_cmd() -> String {
    if cfg!(target_os = "windows") {
        "cmd /c exit 0".to_string()
    } else {
        "true".to_string()
    }
}

/// 一个 立即失败退出 (退出码 1) 的 命令 (跨平台).
fn fail_cmd() -> String {
    if cfg!(target_os = "windows") {
        "cmd /c exit 1".to_string()
    } else {
        "false".to_string()
    }
}

/// 一个 长时间运行 的 命令 (跨平台, 用于 保持 service active).
fn run_cmd() -> String {
    if cfg!(target_os = "windows") {
        "ping -n 60 127.0.0.1".to_string()
    } else {
        "sleep 60".to_string()
    }
}

/// 一个 短时间运行后 成功退出 的 命令 (跨平台, 用于 oneshot 启动后 短暂 active).
fn slow_ok_cmd() -> String {
    if cfg!(target_os = "windows") {
        "cmd /c ping -n 2 127.0.0.1".to_string()
    } else {
        "sleep 1".to_string()
    }
}

/// 一条 同时向 stdout / stderr 各输出一行 的 命令 (跨平台).
fn out_err_cmd() -> String {
    if cfg!(target_os = "windows") {
        "cmd /c echo hello & echo oops 1>&2".to_string()
    } else {
        "sh -c printf${IFS}hello;printf${IFS}oops${IFS}>&2".to_string()
    }
}

/// 带 `ExecReload` 的 额外配置 (跨平台).
fn reload_extra() -> String {
    format!(r#""env:Service.ExecReload" = "{}""#, ok_cmd())
}

/// 把 字符串 转义为 TOML 基本字符串 (转义 `\` 与 `"`).
fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}

/// 单个 unit 的 配置文本 (`ExecStart` 可指定, `extra` 追加其它 `env:` 字段).
fn unit_conf(exec_start: &str, extra: &str) -> String {
    format!(
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "t.service"
"env:Unit.Description" = "test service"
"env:Service.ExecStart" = "{}"
{}"#,
        toml_str(exec_start),
        extra
    )
}

/// 多个 unit 的 配置文本 (每个 unit 指定 名称 与 `Install.WantedBy`).
fn multi_units(units: &[(&str, &str)]) -> String {
    let mut s = String::from("pm_tc = 1\n\n");
    for (name, wanted_by) in units {
        s.push_str(&format!(
            "[[t.sysd]]\n\"env:Unit\" = \"{}\"\n\"env:Service.ExecStart\" = \"{}\"\n\"env:Install.WantedBy\" = \"{}\"\n\n",
            name,
            run_cmd(),
            wanted_by
        ));
    }
    s
}

/// 写入 配置 并 构造 [`SysD`], 返回 (sysd, logd, 临时目录).
fn make_sysd(text: &str) -> (SysD, Logd, TempDir) {
    let (sysd, logd, dir, _exit_rx) = make_sysd_with_exit(text);
    (sysd, logd, dir)
}

/// 写入 配置 并 构造 [`SysD`], 同时返回 退出信号 接收端.
fn make_sysd_with_exit(text: &str) -> (SysD, Logd, TempDir, mpsc::Receiver<Shutdown>) {
    let dir = TempDir::new("sysd");
    let conf_path = dir.path().join("c.toml");
    std::fs::write(&conf_path, text).unwrap();
    let conf_path = conf_path.to_string_lossy().into_owned();

    let conf = conf::load_conf(&conf_path).unwrap();
    let db = DbClient::open(&conf.db_path).unwrap();
    let logd = Logd::new(db.clone());
    let (exit_tx, exit_rx) = mpsc::channel::<Shutdown>(1);
    let sysd = SysD::spawn(db, logd.clone(), &conf_path, conf.units, exit_tx);
    (sysd, logd, dir, exit_rx)
}

/// 执行 一条 ctl 命令, 返回 响应.
async fn handle(sysd: &SysD, cmd: &SdReq) -> SdRes {
    let (tx, rx) = oneshot::channel();
    sysd.send(SysDM::SdReq((cmd.clone(), tx))).await.unwrap();
    rx.await
        .unwrap_or_else(|_| SdRes::Err("sysd stopped".to_string()))
}

/// 把 `SdRes` 转换为 `Result<(), SysdError>`.
fn ok(resp: SdRes) -> Result<(), SysdError> {
    match resp {
        SdRes::Ok => Ok(()),
        SdRes::Err(message) => Err(SysdError::Failed(message)),
        other => panic!("unexpected response: {:?}", other),
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

async fn svc_start(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Start))).await)
}

async fn svc_stop(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Stop))).await)
}

async fn svc_restart(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Restart))).await)
}

async fn svc_try_restart(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::TryRestart))).await)
}

async fn svc_reload(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Reload))).await)
}

async fn svc_reload_or_restart(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::ReloadOrRestart))).await)
}

async fn svc_try_reload_or_restart(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(
        sysd,
        &SdReq::Unit(unit(name, SdReqUnitOp::TryReloadOrRestart)),
    )
    .await)
}

async fn svc_kill(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Kill))).await)
}

async fn svc_reset_failed(sysd: &SysD, name: Option<&str>) -> Result<(), SysdError> {
    ok(handle(
        sysd,
        &SdReq::Unit(unit(name.unwrap_or(""), SdReqUnitOp::ResetFailed)),
    )
    .await)
}

async fn unit_enable(sysd: &SysD, name: &str, now: bool) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Enable { now }))).await)
}

async fn unit_disable(sysd: &SysD, name: &str, now: bool) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Disable { now }))).await)
}

async fn unit_reenable(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Reenable))).await)
}

async fn unit_mask(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Mask))).await)
}

async fn unit_unmask(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Unmask))).await)
}

async fn daemon_reload(sysd: &SysD) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::UnitD(SdReqUnitd::DaemonReload)).await)
}

async fn status(sysd: &SysD, name: &str) -> Result<SdResUnitStatus, SysdError> {
    match handle(sysd, &SdReq::UnitRO(unit_ro(name, SdReqUnitOpRO::Status))).await {
        SdRes::Status(status) => Ok(status),
        SdRes::Err(message) => Err(SysdError::Failed(message)),
        other => panic!("unexpected response: {:?}", other),
    }
}

async fn unit_state(sysd: &SysD, name: &str) -> Option<SdResUnitState> {
    match handle(
        sysd,
        &SdReq::UnitRO(unit_ro(name, SdReqUnitOpRO::UnitState)),
    )
    .await
    {
        SdRes::UnitState(state) => state,
        other => panic!("unexpected response: {:?}", other),
    }
}

async fn list_units(sysd: &SysD) -> Vec<SdResUnitState> {
    match handle(sysd, &SdReq::UnitdRO(SdReqUnitdRO::ListUnits)).await {
        SdRes::ListUnits(units) => units,
        other => panic!("unexpected response: {:?}", other),
    }
}

async fn config(sysd: &SysD, name: &str) -> Result<conf::Unit, SysdError> {
    match handle(sysd, &SdReq::UnitRO(unit_ro(name, SdReqUnitOpRO::Config))).await {
        SdRes::Config(config) => Ok(config),
        SdRes::Err(message) => Err(SysdError::Failed(message)),
        other => panic!("unexpected response: {:?}", other),
    }
}

/// 从 `status` 结果中取出 service 特有状态 (当前版本 status 只可能是 service).
fn service_status(st: &SdResUnitStatus) -> &SdResServiceStatus {
    match &st.s {
        SdResUnitS::Service(s) => s,
        SdResUnitS::Timer(_) => panic!("unexpected timer status"),
    }
}

async fn list_dependencies(sysd: &SysD, name: &str) -> Result<Vec<String>, SysdError> {
    match handle(
        sysd,
        &SdReq::UnitRO(unit_ro(name, SdReqUnitOpRO::ListDependencies)),
    )
    .await
    {
        SdRes::Dependencies(deps) => Ok(deps),
        SdRes::Err(message) => Err(SysdError::Failed(message)),
        other => panic!("unexpected response: {:?}", other),
    }
}

async fn get_default(sysd: &SysD) -> String {
    match handle(sysd, &SdReq::UnitdRO(SdReqUnitdRO::GetDefault)).await {
        SdRes::DefaultTarget(target) => target,
        other => panic!("unexpected response: {:?}", other),
    }
}

async fn set_default(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(
        sysd,
        &SdReq::UnitD(SdReqUnitd::SetDefault {
            target: name.into(),
        }),
    )
    .await)
}

async fn is_active(sysd: &SysD, name: &str) -> bool {
    status(sysd, name).await.unwrap().active == UnitSta::Active
}

async fn is_failed(sysd: &SysD, name: &str) -> bool {
    status(sysd, name).await.unwrap().active == UnitSta::Failed
}

async fn unit_enabled(sysd: &SysD, name: &str) -> bool {
    let s = unit_state(sysd, name).await.unwrap();
    s.enabled && !s.masked
}

async fn unit_masked(sysd: &SysD, name: &str) -> bool {
    unit_state(sysd, name).await.unwrap().masked
}

/// 反复执行 谓词 直到满足 (超时 则 panic).
async fn pump_until<F>(sysd: &SysD, mut pred: F)
where
    F: FnMut(&SysD) -> Pin<Box<dyn Future<Output = bool> + '_>>,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if pred(sysd).await {
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!("monitor condition not reached");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// 反复读取 unit 日志, 直到 同时出现 指定 stdout / stderr 记录 (超时 则 panic).
async fn wait_stdout_stderr(logd: &Logd, unit: &str, out: &str, err: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let entries = logd.recent_entries(Some(unit), 200).await.unwrap();
        let has_out = entries
            .iter()
            .any(|e| e.level == "stdout" && e.msg.contains(out));
        let has_err = entries
            .iter()
            .any(|e| e.level == "stderr" && e.msg.contains(err));
        if has_out && has_err {
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!("stdout/stderr not captured: {:?}", entries);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn start_stop_is_active_status() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    assert!(!is_active(&sysd, "t.service").await);

    svc_start(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);

    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Active);
    assert_eq!(service_status(&st).sub, ServiceSta::Running);
    assert!(service_status(&st).pid.is_some());
    assert!(!st.enabled);
    assert!(!st.masked);

    svc_stop(&sysd, "t.service").await.unwrap();
    assert!(!is_active(&sysd, "t.service").await);

    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Inactive);
    assert_eq!(service_status(&st).sub, ServiceSta::Dead);
    assert!(service_status(&st).pid.is_none());
}

#[tokio::test]
async fn start_idempotent() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();
    svc_start(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

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
    assert_eq!(units[2].unit_type, conf::UnitType::Target);
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
    assert_eq!(c.service.exec_start, vec!["bin", "--x"]);
    assert_eq!(c.service.restart.as_str(), "always");
    assert_eq!(c.service.restart_sec, 3);
    assert_eq!(
        c.service.exec_stop.as_deref(),
        Some(&["stop".to_string()][..])
    );
    assert_eq!(
        c.service.exec_reload.as_deref(),
        Some(&["reload".to_string()][..])
    );
    assert_eq!(c.install.wanted_by.as_deref(), Some("default.target"));
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
async fn reload_and_reload_missing() {
    let text = unit_conf(&run_cmd(), &reload_extra());
    let (sysd, _logd, _dir) = make_sysd(&text);
    svc_start(&sysd, "t.service").await.unwrap();
    svc_reload(&sysd, "t.service").await.unwrap();

    let (sysd2, _logd2, _dir2) = make_sysd(&unit_conf(&run_cmd(), ""));
    assert!(svc_reload(&sysd2, "t.service").await.is_err());
}

#[tokio::test]
async fn restart_active() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();
    svc_restart(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn try_restart() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    svc_try_restart(&sysd, "t.service").await.unwrap();
    assert!(!is_active(&sysd, "t.service").await);

    svc_start(&sysd, "t.service").await.unwrap();
    svc_try_restart(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn reload_or_restart() {
    let text = unit_conf(&ok_cmd(), &reload_extra());
    let (sysd, _logd, _dir) = make_sysd(&text);
    svc_reload_or_restart(&sysd, "t.service").await.unwrap();

    let (sysd2, _logd2, _dir2) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd2, "t.service").await.unwrap();
    svc_reload_or_restart(&sysd2, "t.service").await.unwrap();
    assert!(is_active(&sysd2, "t.service").await);
}

#[tokio::test]
async fn try_reload_or_restart() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_try_reload_or_restart(&sysd, "t.service").await.unwrap();
    assert!(!is_active(&sysd, "t.service").await);

    svc_start(&sysd, "t.service").await.unwrap();
    svc_try_reload_or_restart(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn kill() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();
    svc_kill(&sysd, "t.service").await.unwrap();
    assert!(!is_active(&sysd, "t.service").await);
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

    unit_mask(&sysd, "t.service").await.unwrap();
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
async fn mask_stops_and_blocks_start() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();
    unit_mask(&sysd, "t.service").await.unwrap();
    assert!(!is_active(&sysd, "t.service").await);
    assert!(unit_masked(&sysd, "t.service").await);
    assert!(svc_start(&sysd, "t.service").await.is_err());

    unit_unmask(&sysd, "t.service").await.unwrap();
    svc_start(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
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
    unit_mask(&sysd, "d.service").await.unwrap();

    let (tx, rx) = oneshot::channel();
    sysd.send(SysDM::StartDefaultTarget(tx)).await.unwrap();
    rx.await.unwrap().unwrap();

    assert!(is_active(&sysd, "a.service").await);
    assert!(!is_active(&sysd, "b.service").await);
    assert!(!is_active(&sysd, "c.service").await);
    assert!(!is_active(&sysd, "d.service").await);
}

#[tokio::test]
async fn exit_notifies_shutdown() {
    let text = multi_units(&[
        ("a.service", "default.target"),
        ("b.service", "default.target"),
    ]);
    let (sysd, _logd, _dir, mut exit_rx) = make_sysd_with_exit(&text);

    svc_start(&sysd, "a.service").await.unwrap();
    svc_start(&sysd, "b.service").await.unwrap();
    let resp = handle(&sysd, &SdReq::SysD(SdReqSysd::Exit)).await;
    assert!(matches!(resp, SdRes::Ok));
    assert_eq!(exit_rx.recv().await, Some(Shutdown::Exit));
}

#[tokio::test]
async fn soft_reboot_notifies_shutdown() {
    let text = multi_units(&[
        ("a.service", "default.target"),
        ("b.service", "default.target"),
    ]);
    let (sysd, _logd, _dir, mut exit_rx) = make_sysd_with_exit(&text);

    svc_start(&sysd, "a.service").await.unwrap();
    svc_start(&sysd, "b.service").await.unwrap();
    let resp = handle(&sysd, &SdReq::SysD(SdReqSysd::SoftReboot)).await;
    assert!(matches!(resp, SdRes::Ok));
    assert_eq!(exit_rx.recv().await, Some(Shutdown::SoftReboot));
}

#[tokio::test]
async fn handle_dispatch() {
    let text = unit_conf(&ok_cmd(), r#""env:Install.WantedBy" = "default.target""#);
    let (sysd, _logd, _dir) = make_sysd(&text);
    unit_enable(&sysd, "t.service", false).await.unwrap();

    match handle(&sysd, &SdReq::UnitdRO(SdReqUnitdRO::ListUnits)).await {
        SdRes::ListUnits(units) => assert_eq!(units.len(), 2),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(&sysd, &SdReq::UnitdRO(SdReqUnitdRO::ListTimers)).await {
        SdRes::Timers(timers) => assert!(timers.is_empty()),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(
        &sysd,
        &SdReq::UnitRO(unit_ro("t.service", SdReqUnitOpRO::UnitState)),
    )
    .await
    {
        SdRes::UnitState(state) => {
            assert_eq!(state.as_ref().unwrap().active, UnitSta::Inactive)
        }
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(
        &sysd,
        &SdReq::UnitRO(unit_ro("t.service", SdReqUnitOpRO::Status)),
    )
    .await
    {
        SdRes::Status(status) => assert_eq!(status.name, "t.service"),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(
        &sysd,
        &SdReq::UnitRO(unit_ro("t.service", SdReqUnitOpRO::Config)),
    )
    .await
    {
        SdRes::Config(config) => assert_eq!(config.name, "t.service"),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(
        &sysd,
        &SdReq::UnitRO(unit_ro("t.service", SdReqUnitOpRO::ListDependencies)),
    )
    .await
    {
        SdRes::Dependencies(deps) => assert!(deps.is_empty()),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(&sysd, &SdReq::UnitdRO(SdReqUnitdRO::GetDefault)).await {
        SdRes::DefaultTarget(target) => assert_eq!(target, "default.target"),
        other => panic!("unexpected response: {:?}", other),
    }
    match handle(
        &sysd,
        &SdReq::UnitRO(unit_ro("t.service", SdReqUnitOpRO::UnitState)),
    )
    .await
    {
        SdRes::UnitState(state) => {
            let s = state.unwrap();
            assert!(s.enabled);
            assert!(!s.masked);
        }
        other => panic!("unexpected response: {:?}", other),
    }
}

#[tokio::test]
async fn handle_todo_commands() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    let cmds = vec![
        SdReq::Unit(unit("t.service", SdReqUnitOp::Clean)),
        SdReq::Unit(unit("t.service", SdReqUnitOp::Preset)),
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
async fn oneshot_exits_inactive() {
    let text = unit_conf(&slow_ok_cmd(), r#""env:Service.Type" = "oneshot""#);
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);

    pump_until(&sysd, |s| {
        Box::pin(async move {
            let st = status(s, "t.service").await.unwrap();
            st.active == UnitSta::Inactive && service_status(&st).pid.is_none()
        })
    })
    .await;

    assert!(!is_active(&sysd, "t.service").await);
    assert!(!is_failed(&sysd, "t.service").await);
    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(service_status(&st).exit_status, Some(0));
}

#[tokio::test]
async fn remain_after_exit() {
    let text = unit_conf(&ok_cmd(), r#""env:Service.RemainAfterExit" = "yes""#);
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move {
            let st = status(s, "t.service").await.unwrap();
            service_status(&st).pid.is_none()
        })
    })
    .await;

    assert!(is_active(&sysd, "t.service").await);
    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Active);
    assert_eq!(service_status(&st).sub, ServiceSta::Exited);
    assert_eq!(service_status(&st).exit_status, Some(0));
}

#[tokio::test]
async fn win_create_no_window_starts() {
    let text = unit_conf(&run_cmd(), r#""env:Win.CreateNoWindow" = "yes""#);
    let (sysd, _logd, _dir) = make_sysd(&text);
    svc_start(&sysd, "t.service").await.unwrap();
    assert!(is_active(&sysd, "t.service").await);
}

#[tokio::test]
async fn restart_always() {
    let text = unit_conf(
        &fail_cmd(),
        r#""env:Service.Restart" = "always"
"env:Service.RestartSec" = "0"
"#,
    );
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move {
            let st = status(s, "t.service").await.unwrap();
            service_status(&st).restart_count >= 1
        })
    })
    .await;

    assert!(!is_failed(&sysd, "t.service").await);
}

#[tokio::test]
async fn restart_no_marks_failed() {
    let text = unit_conf(&fail_cmd(), "");
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move {
            let st = status(s, "t.service").await.unwrap();
            st.active == UnitSta::Failed
        })
    })
    .await;

    assert!(is_failed(&sysd, "t.service").await);
    assert!(!is_active(&sysd, "t.service").await);
    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(service_status(&st).exit_status, Some(1));
    assert_eq!(service_status(&st).restart_count, 0);
}

#[tokio::test]
async fn restart_on_failure_success_no_restart() {
    let text = unit_conf(&ok_cmd(), r#""env:Service.Restart" = "on-failure""#);
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move {
            let st = status(s, "t.service").await.unwrap();
            service_status(&st).pid.is_none()
        })
    })
    .await;

    assert!(!is_failed(&sysd, "t.service").await);
    let st = status(&sysd, "t.service").await.unwrap();
    assert_eq!(service_status(&st).restart_count, 0);
}

#[tokio::test]
async fn restart_on_failure_restarts() {
    let text = unit_conf(
        &fail_cmd(),
        r#""env:Service.Restart" = "on-failure"
"env:Service.RestartSec" = "0"
"#,
    );
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move {
            let st = status(s, "t.service").await.unwrap();
            service_status(&st).restart_count >= 1
        })
    })
    .await;

    assert!(!is_failed(&sysd, "t.service").await);
}

#[tokio::test]
async fn reset_failed() {
    let text = unit_conf(&fail_cmd(), "");
    let (sysd, _logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move { is_failed(s, "t.service").await })
    })
    .await;
    assert!(is_failed(&sysd, "t.service").await);
    svc_reset_failed(&sysd, Some("t.service")).await.unwrap();
    assert!(!is_failed(&sysd, "t.service").await);

    svc_start(&sysd, "t.service").await.unwrap();
    pump_until(&sysd, |s| {
        Box::pin(async move { is_failed(s, "t.service").await })
    })
    .await;
    assert!(is_failed(&sysd, "t.service").await);
    svc_reset_failed(&sysd, None).await.unwrap();
    assert!(!is_failed(&sysd, "t.service").await);
}

#[tokio::test]
async fn reset_failed_never_run_silent_success() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    svc_reset_failed(&sysd, Some("t.service")).await.unwrap();
    assert!(!is_failed(&sysd, "t.service").await);
}

#[tokio::test]
async fn reset_failed_unknown_unit() {
    let (sysd, _logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));
    assert!(svc_reset_failed(&sysd, Some("nope.service")).await.is_err());
}

#[tokio::test]
async fn unit_state_states() {
    let text = unit_conf(&run_cmd(), "");
    let (sysd, _logd, _dir) = make_sysd(&text);

    let st = unit_state(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Inactive);
    assert_eq!(st.sub, UnitSubSta::Service(ServiceSta::Dead));
    assert!(unit_state(&sysd, "nope.service").await.is_none());

    svc_start(&sysd, "t.service").await.unwrap();
    let st = unit_state(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Active);

    svc_stop(&sysd, "t.service").await.unwrap();
    let st = unit_state(&sysd, "t.service").await.unwrap();
    assert_eq!(st.active, UnitSta::Inactive);
}

#[tokio::test]
async fn handle_start_uses_job_queue() {
    let text = unit_conf(&run_cmd(), "");
    let (sysd, _logd, _dir) = make_sysd(&text);

    match handle(&sysd, &SdReq::Unit(unit("t.service", SdReqUnitOp::Start))).await {
        SdRes::Ok => {}
        other => panic!("unexpected response: {:?}", other),
    }
    assert!(is_active(&sysd, "t.service").await);
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

    // list-units 只列出 当前配置 (不再有 legacy).
    let units = list_units(&sysd).await;
    assert!(!units.iter().any(|u| u.name == "t.service"));

    // 停止后 不再 active (已从配置移除, status 不再可查).
    svc_stop(&sysd, "t.service").await.unwrap();
    assert!(status(&sysd, "t.service").await.is_err());
}

#[tokio::test]
async fn captures_stdout_stderr() {
    let (sysd, logd, _dir) = make_sysd(&unit_conf(&out_err_cmd(), ""));
    svc_start(&sysd, "t.service").await.unwrap();
    wait_stdout_stderr(&logd, "t.service", "hello", "oops").await;
}

#[tokio::test]
async fn exec_reload_captures_output() {
    let text = format!(
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "t.service"
"env:Unit.Description" = "test service"
"env:Service.ExecStart" = "{}"
"env:Service.ExecReload" = "{}"
"#,
        toml_str(&run_cmd()),
        toml_str(&out_err_cmd())
    );
    let (sysd, logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    svc_reload(&sysd, "t.service").await.unwrap();
    wait_stdout_stderr(&logd, "t.service", "hello", "oops").await;
}

#[tokio::test]
async fn exec_stop_captures_output() {
    let text = format!(
        r#"pm_tc = 1

[[t.sysd]]
"env:Unit" = "t.service"
"env:Unit.Description" = "test service"
"env:Service.ExecStart" = "{}"
"env:Service.ExecStop" = "{}"
"#,
        toml_str(&run_cmd()),
        toml_str(&out_err_cmd())
    );
    let (sysd, logd, _dir) = make_sysd(&text);

    svc_start(&sysd, "t.service").await.unwrap();
    svc_stop(&sysd, "t.service").await.unwrap();
    wait_stdout_stderr(&logd, "t.service", "hello", "oops").await;
}

#[tokio::test]
async fn self_actions_log_without_unit() {
    let (sysd, logd, _dir) = make_sysd(&unit_conf(&run_cmd(), ""));

    daemon_reload(&sysd).await.unwrap();
    handle(&sysd, &SdReq::SysD(SdReqSysd::Exit)).await;
    logd.flush().unwrap();

    let entries = logd.recent_entries(None, 200).await.unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e.unit.is_none() && e.msg == "daemon-reload")
    );
    assert!(
        entries
            .iter()
            .any(|e| e.unit.is_none() && e.msg == "exit requested")
    );
}
