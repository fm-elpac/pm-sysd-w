//! sysd 集成测试 公共工具 (通过公开句柄 `SysD` 驱动, 供各下级模块测试复用).

use std::{future::Future, pin::Pin};

use tokio::sync::oneshot;

use crate::{
    conf::{self, limit::SysdLimit},
    jsonl::{
        SdReq, SdReqPagePos, SdReqUnit, SdReqUnitList, SdReqUnitOp, SdReqUnitOpRO, SdReqUnitRO,
        SdReqUnitd, SdReqUnitdRO, SdRes, SdResServiceStatus, SdResUnitS, SdResUnitState,
        SdResUnitStatus, SdResWhoamiInit,
    },
    logd::Logd,
    sysu::{EnableSta, LoadSta, UnitSta},
    test::TempDir,
};

use super::{
    Server, SysD, SysDM, SysdError,
    init::Shutdown,
    reload::{ReloadR, Reloader, assemble},
};

/// 测试用 Reloader: 直接解析配置文件, 不启动 `pm-sysd-init --reload` 子进程.
pub struct TestReloader;

impl Reloader for TestReloader {
    fn reload<'a>(
        &'a self,
        db: Option<crate::db::DbClient>,
        conf_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(ReloadR, crate::db::DbClient), SysdError>> + Send + 'a>>
    {
        Box::pin(async move {
            let limit = SysdLimit::default();
            let conf = crate::conf::load_conf(conf_path, &limit)
                .map_err(|e| SysdError::Failed(e.to_string()))?;
            assemble(db, conf, &limit).await
        })
    }
}

/// 一个 立即成功退出 (退出码 0) 的 命令 (跨平台).
pub fn ok_cmd() -> String {
    if cfg!(target_os = "windows") {
        "cmd /c exit 0".to_string()
    } else {
        "true".to_string()
    }
}

/// 一个 立即失败退出 (退出码 1) 的 命令 (跨平台).
pub fn fail_cmd() -> String {
    if cfg!(target_os = "windows") {
        "cmd /c exit 1".to_string()
    } else {
        "false".to_string()
    }
}

/// 一个 长时间运行 的 命令 (跨平台, 用于 保持 service active).
pub fn run_cmd() -> String {
    if cfg!(target_os = "windows") {
        "ping -n 60 127.0.0.1".to_string()
    } else {
        "sleep 60".to_string()
    }
}

/// 一个 短时间运行后 成功退出 的 命令 (跨平台, 用于 oneshot 启动后 短暂 active).
pub fn slow_ok_cmd() -> String {
    if cfg!(target_os = "windows") {
        "cmd /c ping -n 2 127.0.0.1".to_string()
    } else {
        "sleep 1".to_string()
    }
}

/// 一条 同时向 stdout / stderr 各输出一行 的 命令 (跨平台).
pub fn out_err_cmd() -> String {
    if cfg!(target_os = "windows") {
        "cmd /c echo hello & echo oops 1>&2".to_string()
    } else {
        "sh -c printf${IFS}hello;printf${IFS}oops${IFS}>&2".to_string()
    }
}

/// 带 `ExecReload` 的 额外配置 (跨平台).
pub fn reload_extra() -> String {
    format!(r#""env:Service.ExecReload" = "{}""#, ok_cmd())
}

/// 把 字符串 转义为 TOML 基本字符串 (转义 `\` 与 `"`).
pub fn toml_str(s: &str) -> String {
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
pub fn unit_conf(exec_start: &str, extra: &str) -> String {
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
pub fn multi_units(units: &[(&str, &str)]) -> String {
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
pub fn make_sysd(text: &str) -> (SysD, Logd, TempDir) {
    let (sysd, logd, dir, _exit_rx) = make_sysd_with_exit(text);
    (sysd, logd, dir)
}

/// 写入 配置 并 构造 [`SysD`], 同时返回 退出信号 接收端.
pub fn make_sysd_with_exit(
    text: &str,
) -> (
    SysD,
    Logd,
    TempDir,
    oneshot::Receiver<Result<Shutdown, SysdError>>,
) {
    let dir = TempDir::new("sysd");
    let conf_path = dir.path().join("c.toml");
    std::fs::write(&conf_path, text).unwrap();
    let conf_path = conf_path.to_string_lossy().into_owned();

    // 在独立线程的 runtime 中执行异步加载 (测试辅助, 不经过生产 subprocess 路径).
    let load_path = conf_path.clone();
    let (load, db) = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move { TestReloader.reload(None, &load_path).await })
    })
    .join()
    .unwrap()
    .unwrap();

    let logd = Logd::new(db.clone(), &SysdLimit::default());

    // 构造一个只用于捕获退出结果的 Server (不绑定 IPC, 也不接受连接).
    let instance = crate::pa::Instance::Machine(format!("sysd_test_{}", std::process::id()));
    let (server, sender) = Server::new(instance, SysdLimit::default());
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<Result<Shutdown, SysdError>>();
    tokio::spawn(async move {
        let r = server.loop_accept().await;
        let _ = shutdown_tx.send(r);
    });

    let whoami = SdResWhoamiInit {
        instance_type: "machine".to_string(),
        instance_name: "sysd_test".to_string(),
        pid: std::process::id(),
        service_name: None,
    };

    let sysd = SysD::spawn(
        db,
        logd.clone(),
        &conf_path,
        load,
        sender,
        Box::new(TestReloader),
        whoami,
        &SysdLimit::default(),
    );
    (sysd, logd, dir, shutdown_rx)
}

/// 执行 一条 ctl 命令, 返回 响应.
pub async fn handle(sysd: &SysD, cmd: &SdReq) -> SdRes {
    let (tx, rx) = oneshot::channel();
    sysd.send(SysDM::SdReq(cmd.clone(), tx)).await.unwrap();
    rx.await
        .unwrap_or_else(|_| SdRes::Err("sysd stopped".to_string()))
}

/// 把 `SdRes` 转换为 `Result<(), SysdError>`.
pub fn ok(resp: SdRes) -> Result<(), SysdError> {
    match resp {
        SdRes::Ok(_) => Ok(()),
        SdRes::Err(message) => Err(SysdError::Failed(message)),
        other => panic!("unexpected response: {:?}", other),
    }
}

/// 构造 针对 单个 unit 的 修改 请求.
pub fn unit(name: &str, op: SdReqUnitOp) -> SdReqUnit {
    SdReqUnit {
        name: name.to_string(),
        op,
    }
}

/// 构造 针对 单个 unit 的 只读查询 请求.
pub fn unit_ro(name: &str, op: SdReqUnitOpRO) -> SdReqUnitRO {
    SdReqUnitRO {
        name: name.to_string(),
        op,
    }
}

pub async fn svc_start(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Start))).await)
}

pub async fn svc_stop(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Stop))).await)
}

pub async fn svc_restart(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Restart))).await)
}

pub async fn svc_try_restart(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::TryRestart))).await)
}

pub async fn svc_reload(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Reload))).await)
}

pub async fn svc_reload_or_restart(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::ReloadOrRestart))).await)
}

pub async fn svc_try_reload_or_restart(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(
        sysd,
        &SdReq::Unit(unit(name, SdReqUnitOp::TryReloadOrRestart)),
    )
    .await)
}

pub async fn svc_kill(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Kill))).await)
}

pub async fn svc_reset_failed(sysd: &SysD, name: Option<&str>) -> Result<(), SysdError> {
    ok(handle(
        sysd,
        &SdReq::Unit(unit(name.unwrap_or(""), SdReqUnitOp::ResetFailed)),
    )
    .await)
}

pub async fn unit_enable(sysd: &SysD, name: &str, now: bool) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Enable { now }))).await)
}

pub async fn unit_disable(sysd: &SysD, name: &str, now: bool) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Disable { now }))).await)
}

pub async fn unit_reenable(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Reenable))).await)
}

pub async fn unit_mask(sysd: &SysD, name: &str, now: bool) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Mask { now }))).await)
}

pub async fn unit_unmask(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Unmask))).await)
}

pub async fn unit_preset(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::Unit(unit(name, SdReqUnitOp::Preset))).await)
}

pub async fn daemon_reload(sysd: &SysD) -> Result<(), SysdError> {
    ok(handle(sysd, &SdReq::UnitD(SdReqUnitd::DaemonReload)).await)
}

pub async fn status(sysd: &SysD, name: &str) -> Result<SdResUnitStatus, SysdError> {
    match handle(sysd, &SdReq::UnitRO(unit_ro(name, SdReqUnitOpRO::Status))).await {
        SdRes::Status(status) => Ok(status),
        SdRes::Err(message) => Err(SysdError::Failed(message)),
        other => panic!("unexpected response: {:?}", other),
    }
}

pub async fn unit_state(sysd: &SysD, name: &str) -> Option<SdResUnitState> {
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

pub async fn list_units(sysd: &SysD) -> Vec<SdResUnitState> {
    let mut cursor: Option<String> = None;
    let mut all = Vec::new();
    loop {
        let resp = handle(
            sysd,
            &SdReq::UnitdRO(SdReqUnitdRO::ListUnits(SdReqUnitList {
                limit: u32::MAX,
                cursor: cursor.clone(),
            })),
        )
        .await;
        match resp {
            SdRes::ListUnits(page) => {
                let count = page.list.len();
                all.extend(page.list);
                cursor = page.cursor;
                if count == 0 || cursor.is_none() {
                    return all;
                }
            }
            other => panic!("unexpected response: {:?}", other),
        }
    }
}

pub async fn config(sysd: &SysD, name: &str) -> Result<conf::Unit, SysdError> {
    match handle(sysd, &SdReq::UnitRO(unit_ro(name, SdReqUnitOpRO::Config))).await {
        SdRes::Config(config) => Ok(config),
        SdRes::Err(message) => Err(SysdError::Failed(message)),
        other => panic!("unexpected response: {:?}", other),
    }
}

/// 从 `status` 结果中取出 service 特有状态 (当前版本 status 只可能是 service).
pub fn service_status(st: &SdResUnitStatus) -> &SdResServiceStatus {
    match &st.s {
        SdResUnitS::Service(s) => s,
        SdResUnitS::Timer(_) => panic!("unexpected timer status"),
    }
}

pub async fn list_dependencies(sysd: &SysD, name: &str) -> Result<Vec<String>, SysdError> {
    match handle(
        sysd,
        &SdReq::UnitRO(unit_ro(name, SdReqUnitOpRO::ListDependencies)),
    )
    .await
    {
        SdRes::Dependencies(page) => Ok(page.list),
        SdRes::Err(message) => Err(SysdError::Failed(message)),
        other => panic!("unexpected response: {:?}", other),
    }
}

pub async fn get_default(sysd: &SysD) -> String {
    match handle(sysd, &SdReq::UnitdRO(SdReqUnitdRO::GetDefault)).await {
        SdRes::DefaultTarget(target) => target,
        other => panic!("unexpected response: {:?}", other),
    }
}

pub async fn set_default(sysd: &SysD, name: &str) -> Result<(), SysdError> {
    ok(handle(
        sysd,
        &SdReq::UnitD(SdReqUnitd::SetDefault {
            target: name.into(),
        }),
    )
    .await)
}

pub async fn is_active(sysd: &SysD, name: &str) -> bool {
    status(sysd, name).await.unwrap().active == UnitSta::Active
}

pub async fn is_failed(sysd: &SysD, name: &str) -> bool {
    status(sysd, name).await.unwrap().active == UnitSta::Failed
}

pub async fn unit_enabled(sysd: &SysD, name: &str) -> bool {
    unit_state(sysd, name).await.unwrap().enable == EnableSta::Enabled
}

pub async fn unit_masked(sysd: &SysD, name: &str) -> bool {
    unit_state(sysd, name).await.unwrap().load == LoadSta::Masked
}

/// 反复执行 谓词 直到满足 (超时 则 panic).
pub async fn pump_until<F>(sysd: &SysD, mut pred: F)
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
pub async fn wait_stdout_stderr(logd: &Logd, unit: &str, out: &str, err: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let entries = logd
            .page_entries(None, Some(unit), false, SdReqPagePos::End(0), 200)
            .await
            .unwrap()
            .entries;
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
