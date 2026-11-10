//! 服务管理 核心逻辑.
//!
//! 管理由 pm-sysd-init 启动的 进程 (service). 初版只支持 service 一种 unit 类型.

mod error;
#[cfg(test)]
mod test;

pub use error::SysdError;

use std::{
    collections::HashMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Child,
    sync::Mutex,
};

use crate::{
    conf::{RestartPolicy, ServiceType, Unit},
    db::Db,
    jsonl::{
        ActiveState, Command, EnableState, LogEntry, Response, SubState, UnitConfig, UnitInfo,
        UnitStatus,
    },
    logd::Logd,
    pa::{self, Pa},
};

/// 单个 service 的 运行状态.
#[derive(Debug)]
struct Proc {
    /// 运行中的 子进程 (可能为空).
    child: Option<Child>,
    /// 是否视为 active.
    active: bool,
    /// 是否处于 failed 状态.
    failed: bool,
    /// 用户 是否请求了 停止 (停止后 不再自动重启).
    stop_requested: bool,
    /// 自动重启 次数.
    restart_count: u32,
    /// 计划 下次重启 的时刻 (等待 RestartSec).
    next_restart: Option<Instant>,
    /// 最后一次 退出状态码.
    exit_status: Option<i32>,
}

/// 服务管理器 内部状态.
#[derive(Debug, Default)]
struct State {
    /// 已加载的 unit (名称 -> 配置).
    units: HashMap<String, Unit>,
    /// 运行状态 (名称 -> 状态).
    procs: HashMap<String, Proc>,
}

/// 服务管理器 (类似 systemd 的 PID 1 角色, 但不是真的 PID 1).
///
/// `P` 为 平台抽象 的具体类型 (编译时确定).
pub struct Sysd<P: Pa + 'static> {
    /// 平台抽象.
    pa: Arc<P>,
    /// 数据库.
    db: Arc<Db>,
    /// 日志.
    logd: Logd,
    /// 主配置文件 路径.
    conf_path: String,
    /// 共享状态.
    state: Arc<Mutex<State>>,
}

impl<P: Pa + 'static> Sysd<P> {
    /// 创建服务管理器.
    pub fn new(pa: Arc<P>, db: Arc<Db>, conf_path: &str, units: Vec<Unit>) -> Self {
        let logd = Logd::new(db.clone());
        let units_map = units.into_iter().map(|u| (u.name.clone(), u)).collect();
        Self {
            pa,
            db,
            logd,
            conf_path: conf_path.to_string(),
            state: Arc::new(Mutex::new(State {
                units: units_map,
                procs: HashMap::new(),
            })),
        }
    }

    /// 写入一条日志 (`unit` 可为空, 表示 pm-sysd-init 自身).
    pub fn log(&self, level: &str, unit: Option<&str>, msg: &str) {
        self.logd.write_with(level, unit, msg).ok();
    }

    /// 等待 待写入的日志 全部落库 (优雅退出 / 测试用).
    pub fn flush_logs(&self) -> Result<(), SysdError> {
        self.logd
            .flush()
            .map_err(|e| SysdError::Failed(e.to_string()))
    }

    /// 启动 后台 监控任务 (检测进程退出, 应用重启策略).
    pub fn spawn_monitor(&self) {
        let state = self.state.clone();
        let logd = self.logd.clone();
        let pa = self.pa.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(200));
            loop {
                tick.tick().await;
                monitor_tick(&state, &logd, &pa).await;
            }
        });
    }

    /// 查询 一个 unit 是否 active.
    pub async fn is_active(&self, name: &str) -> Result<bool, SysdError> {
        let s = self.state.lock().await;
        Ok(s.procs.get(name).map(|p| p.active).unwrap_or(false))
    }

    /// 查询 一个 unit 是否 failed.
    pub async fn is_failed(&self, name: &str) -> Result<bool, SysdError> {
        let s = self.state.lock().await;
        Ok(s.procs.get(name).map(|p| p.failed).unwrap_or(false))
    }

    /// 启动一个 service.
    pub async fn start(&self, name: &str) -> Result<(), SysdError> {
        if self.db.unit_get_masked(name).map_err(db_err)? {
            return Err(SysdError::Failed(format!("Unit {} is masked.", name)));
        }

        let mut s = self.state.lock().await;
        let unit = s
            .units
            .get(name)
            .cloned()
            .ok_or_else(|| SysdError::NotFound(format!("Unit {} not found.", name)))?;

        if let Some(p) = s.procs.get(name) {
            if p.child.is_some() || p.active {
                // 已 active: 视为 幂等 成功 (与 systemctl start 风格一致).
                return Ok(());
            }
        }

        self.log("info", Some(name), "starting");
        let child = spawn_child(self.pa.as_ref(), &unit, &self.logd)?;
        let proc = Proc {
            child: Some(child),
            active: true,
            failed: false,
            stop_requested: false,
            restart_count: 0,
            next_restart: None,
            exit_status: None,
        };
        s.procs.insert(name.to_string(), proc);
        Ok(())
    }

    /// 停止一个 service.
    pub async fn stop(&self, name: &str) -> Result<(), SysdError> {
        let (child, unit) = {
            let mut s = self.state.lock().await;
            if !s.units.contains_key(name) {
                return Err(SysdError::NotFound(format!("Unit {} not found.", name)));
            }
            let unit = s.units.get(name).cloned();
            let Some(p) = s.procs.get_mut(name) else {
                return Ok(());
            };
            if p.child.is_none() && !p.active {
                return Ok(());
            }
            p.stop_requested = true;
            p.active = false;
            p.next_restart = None;
            (p.child.take(), unit)
        };

        if let Some(mut c) = child {
            let _ = c.kill().await;
            let _ = c.wait().await;
        }

        // 若配置了 ExecStop, 运行之 (其 stdout/stderr 同样计入日志).
        if let Some(u) = unit.as_ref() {
            if let Some(cmd) = &u.service.exec_stop {
                if let Err(e) = run_exec(self.pa.as_ref(), cmd, u, name, &self.logd).await {
                    self.log("error", Some(name), &format!("ExecStop: {}", e));
                }
            }
        }

        self.log("info", Some(name), "stopped");
        Ok(())
    }

    /// 重启一个 service (先停止, 再启动).
    pub async fn restart(&self, name: &str) -> Result<(), SysdError> {
        self.stop(name).await?;
        self.start(name).await
    }

    /// 仅在 active 时 重启.
    pub async fn try_restart(&self, name: &str) -> Result<(), SysdError> {
        if self.is_active(name).await? {
            self.restart(name).await
        } else {
            Ok(())
        }
    }

    /// 执行 reload (ExecReload).
    pub async fn reload(&self, name: &str) -> Result<(), SysdError> {
        let unit = self.get_unit(name).await?;
        let cmd = unit
            .service
            .exec_reload
            .clone()
            .ok_or_else(|| SysdError::Failed(format!("Unit {} has no ExecReload.", name)))?;
        run_exec(self.pa.as_ref(), &cmd, &unit, name, &self.logd).await
    }

    /// 有 ExecReload 则 reload, 否则 restart.
    pub async fn reload_or_restart(&self, name: &str) -> Result<(), SysdError> {
        let unit = self.get_unit(name).await?;
        if unit.service.exec_reload.is_some() {
            self.reload(name).await
        } else {
            self.restart(name).await
        }
    }

    /// 仅在 active 时 执行 reload-or-restart.
    pub async fn try_reload_or_restart(&self, name: &str) -> Result<(), SysdError> {
        if self.is_active(name).await? {
            self.reload_or_restart(name).await
        } else {
            Ok(())
        }
    }

    /// 强制终止 一个 service 的进程.
    pub async fn kill(&self, name: &str) -> Result<(), SysdError> {
        let child = {
            let mut s = self.state.lock().await;
            if !s.units.contains_key(name) {
                return Err(SysdError::NotFound(format!("Unit {} not found.", name)));
            }
            let Some(p) = s.procs.get_mut(name) else {
                return Ok(());
            };
            p.stop_requested = true;
            p.active = false;
            p.child.take()
        };
        if let Some(mut c) = child {
            let _ = c.kill().await;
            let _ = c.wait().await;
        }
        Ok(())
    }

    /// 清除 failed 状态 (可选 指定 unit; 不指定则 清除全部).
    pub async fn reset_failed(&self, name: Option<&str>) -> Result<(), SysdError> {
        match name {
            Some(n) => {
                let mut s = self.state.lock().await;
                let p = s
                    .procs
                    .get_mut(n)
                    .ok_or_else(|| SysdError::NotFound(format!("Unit {} not found.", n)))?;
                p.failed = false;
                Ok(())
            }
            None => {
                let mut s = self.state.lock().await;
                for p in s.procs.values_mut() {
                    p.failed = false;
                }
                Ok(())
            }
        }
    }

    /// 查询一个 service 的 结构化状态.
    pub async fn status(&self, name: &str) -> Result<UnitStatus, SysdError> {
        let (unit, active, sub, pid, restart_count, exit_status) = {
            let s = self.state.lock().await;
            let unit = s
                .units
                .get(name)
                .cloned()
                .ok_or_else(|| SysdError::NotFound(format!("Unit {} not found.", name)))?;
            let p = s.procs.get(name);
            let (active, sub) = active_sub(p);
            let pid = p.and_then(|p| p.child.as_ref()).and_then(|c| c.id());
            let restart_count = p.map(|p| p.restart_count).unwrap_or(0);
            let exit_status = p.and_then(|p| p.exit_status);
            (unit, active, sub, pid, restart_count, exit_status)
        };

        let enabled = self.db.unit_get_enabled(name).map_err(db_err)?;
        let masked = self.db.unit_get_masked(name).map_err(db_err)?;

        Ok(UnitStatus {
            name: unit.name,
            description: unit.description,
            enabled,
            masked,
            active,
            sub,
            pid,
            restart_count,
            exit_status,
        })
    }

    /// 列出 所有 unit 的 运行状态.
    pub async fn list_units(&self) -> Result<Vec<UnitInfo>, SysdError> {
        let s = self.state.lock().await;
        let mut names: Vec<String> = s.units.keys().cloned().collect();
        names.sort();

        let mut out = Vec::new();
        for n in &names {
            let u = &s.units[n];
            let p = s.procs.get(n);
            let (active, sub) = active_sub(p);
            out.push(UnitInfo {
                name: u.name.clone(),
                description: u.description.clone(),
                active,
                sub,
            });
        }
        Ok(out)
    }

    /// 读取 unit 配置 (结构化).
    pub async fn unit_config(&self, name: &str) -> Result<UnitConfig, SysdError> {
        let unit = self.get_unit(name).await?;
        Ok(UnitConfig {
            name: unit.name,
            description: unit.description,
            service_type: unit.service.service_type.as_str().to_string(),
            exec_start: unit.service.exec_start,
            exec_stop: unit.service.exec_stop,
            exec_reload: unit.service.exec_reload,
            restart: unit.service.restart.as_str().to_string(),
            restart_sec: unit.service.restart_sec,
            wanted_by: unit.install.wanted_by,
        })
    }

    /// 列出依赖 (初版: 仅 Install.WantedBy).
    pub async fn list_dependencies(&self, name: &str) -> Result<Vec<String>, SysdError> {
        let unit = self.get_unit(name).await?;
        Ok(unit.install.wanted_by.into_iter().collect())
    }

    /// enable (可选 同时 start).
    pub async fn enable(&self, name: &str, now: bool) -> Result<(), SysdError> {
        let _ = self.get_unit(name).await?;
        if self.db.unit_get_masked(name).map_err(db_err)? {
            return Err(SysdError::Failed(format!("Unit {} is masked.", name)));
        }
        self.db.unit_set_enabled(name, true).map_err(db_err)?;
        self.logd.write_with("info", Some(name), "enabled").ok();
        if now {
            self.start(name).await?;
        }
        Ok(())
    }

    /// disable (可选 同时 stop).
    pub async fn disable(&self, name: &str, now: bool) -> Result<(), SysdError> {
        let _ = self.get_unit(name).await?;
        self.db.unit_set_enabled(name, false).map_err(db_err)?;
        self.logd.write_with("info", Some(name), "disabled").ok();
        if now {
            self.stop(name).await?;
        }
        Ok(())
    }

    /// reenable (先 disable, 再 enable).
    pub async fn reenable(&self, name: &str) -> Result<(), SysdError> {
        self.disable(name, false).await?;
        self.enable(name, false).await
    }

    /// 查询 enable 状态.
    pub async fn enable_state(&self, name: &str) -> Result<EnableState, SysdError> {
        if self.db.unit_get_masked(name).map_err(db_err)? {
            Ok(EnableState::Masked)
        } else if self.db.unit_get_enabled(name).map_err(db_err)? {
            Ok(EnableState::Enabled)
        } else {
            Ok(EnableState::Disabled)
        }
    }

    /// mask (若 正在运行, 先停止).
    pub async fn mask(&self, name: &str) -> Result<(), SysdError> {
        let _ = self.stop(name).await;
        self.db.unit_set_masked(name, true).map_err(db_err)?;
        Ok(())
    }

    /// unmask.
    pub async fn unmask(&self, name: &str) -> Result<(), SysdError> {
        self.db.unit_set_masked(name, false).map_err(db_err)?;
        Ok(())
    }

    /// 读取 默认 target.
    pub async fn get_default(&self) -> Result<String, SysdError> {
        let t = self
            .db
            .meta_get("default_target")
            .map_err(db_err)?
            .unwrap_or_else(|| "default.target".to_string());
        Ok(t)
    }

    /// 设置 默认 target.
    pub async fn set_default(&self, name: &str) -> Result<(), SysdError> {
        self.db.meta_set("default_target", name).map_err(db_err)?;
        Ok(())
    }

    /// 重新加载 配置文件.
    pub async fn daemon_reload(&self) -> Result<(), SysdError> {
        let conf = crate::conf::load_conf(&self.conf_path)
            .map_err(|e| SysdError::Failed(e.to_string()))?;
        let mut s = self.state.lock().await;
        s.units = conf
            .units
            .into_iter()
            .map(|u| (u.name.clone(), u))
            .collect();
        self.log("info", None, "daemon-reload");
        Ok(())
    }

    /// 启动 默认 target (`default.target`) 中 已 enable 的 service.
    ///
    /// 由 pm-sysd-init 启动后调用.
    pub async fn start_default_target(&self) -> Result<(), SysdError> {
        let names = self.unit_names().await;
        for n in names {
            let unit = self.get_unit(&n).await?;
            if unit.install.wanted_by.as_deref() != Some("default.target") {
                continue;
            }
            let enabled = self.db.unit_get_enabled(&n).map_err(db_err)?;
            let masked = self.db.unit_get_masked(&n).map_err(db_err)?;
            if !enabled || masked {
                continue;
            }
            if let Err(e) = self.start(&n).await {
                log::error!("start `{}`: {}", n, e);
            }
        }
        Ok(())
    }

    /// 停止 所有 service (soft-reboot / exit 用).
    pub async fn stop_all(&self) -> Result<(), SysdError> {
        let names = self.unit_names().await;
        for n in names {
            let _ = self.stop(&n).await;
        }
        Ok(())
    }

    /// 处理 一条 ctl 命令, 返回 响应.
    pub async fn handle(&self, cmd: &Command) -> Response {
        match self.handle_inner(cmd).await {
            Ok(resp) => resp,
            Err(e) => Response::Err {
                message: e.to_string(),
            },
        }
    }

    /// 处理 命令 的 具体逻辑.
    async fn handle_inner(&self, cmd: &Command) -> Result<Response, SysdError> {
        use Command as C;
        use Response as R;

        Ok(match cmd {
            C::ListUnits => R::ListUnits {
                units: self.list_units().await?,
            },
            C::ListTimers => R::Timers { timers: vec![] },
            C::Logs { query } => R::Logs {
                logs: self
                    .logd
                    .recent_entries(query.unit.as_deref(), query.limit)
                    .map_err(|e| SysdError::Failed(e.to_string()))?
                    .into_iter()
                    .map(|e| LogEntry {
                        id: e.id,
                        ts: e.ts,
                        level: e.level,
                        unit: e.unit,
                        msg: e.msg,
                    })
                    .collect(),
            },
            C::IsActive { unit } => R::Bool {
                value: self.is_active(unit).await?,
            },
            C::IsFailed { unit } => R::Bool {
                value: self.is_failed(unit).await?,
            },
            C::Status { unit } => R::Status {
                status: self.status(unit).await?,
            },
            C::Show { unit } | C::Cat { unit } => R::Config {
                config: self.unit_config(unit).await?,
            },
            C::ListDependencies { unit } => R::Dependencies {
                deps: self.list_dependencies(unit).await?,
            },
            C::Start { unit } => {
                self.start(unit).await?;
                R::Ok
            }
            C::Stop { unit } => {
                self.stop(unit).await?;
                R::Ok
            }
            C::Reload { unit } => {
                self.reload(unit).await?;
                R::Ok
            }
            C::Restart { unit } => {
                self.restart(unit).await?;
                R::Ok
            }
            C::TryRestart { unit } => {
                self.try_restart(unit).await?;
                R::Ok
            }
            C::ReloadOrRestart { unit } => {
                self.reload_or_restart(unit).await?;
                R::Ok
            }
            C::TryReloadOrRestart { unit } => {
                self.try_reload_or_restart(unit).await?;
                R::Ok
            }
            C::Kill { unit } => {
                self.kill(unit).await?;
                R::Ok
            }
            C::Clean { .. } => R::Err {
                message: "TODO: clean".to_string(),
            },
            C::ResetFailed { unit } => {
                self.reset_failed(unit.as_deref()).await?;
                R::Ok
            }
            C::Enable { unit, now } => {
                self.enable(unit, *now).await?;
                R::Ok
            }
            C::Disable { unit, now } => {
                self.disable(unit, *now).await?;
                R::Ok
            }
            C::Reenable { unit } => {
                self.reenable(unit).await?;
                R::Ok
            }
            C::Preset { .. } => R::Err {
                message: "TODO: preset".to_string(),
            },
            C::IsEnabled { unit } => R::EnableState {
                state: self.enable_state(unit).await?,
            },
            C::Mask { unit } => {
                self.mask(unit).await?;
                R::Ok
            }
            C::Unmask { unit } => {
                self.unmask(unit).await?;
                R::Ok
            }
            C::GetDefault => R::DefaultTarget {
                target: self.get_default().await?,
            },
            C::SetDefault { target } => {
                self.set_default(target).await?;
                R::Ok
            }
            C::ShowEnvironment => R::Err {
                message: "TODO: show-environment".to_string(),
            },
            C::SetEnvironment => R::Err {
                message: "TODO: set-environment".to_string(),
            },
            C::UnsetEnvironment => R::Err {
                message: "TODO: unset-environment".to_string(),
            },
            C::ImportEnvironment => R::Err {
                message: "TODO: import-environment".to_string(),
            },
            C::DaemonReload => {
                self.daemon_reload().await?;
                R::Ok
            }
            C::SoftReboot => {
                self.log("info", None, "soft-reboot requested");
                self.stop_all().await?;
                R::Ok
            }
            C::Exit => {
                self.log("info", None, "exit requested");
                self.stop_all().await?;
                R::Ok
            }
        })
    }

    /// 读取 所有 unit 名称.
    async fn unit_names(&self) -> Vec<String> {
        let s = self.state.lock().await;
        s.units.keys().cloned().collect()
    }

    /// 读取 一个 unit 配置.
    async fn get_unit(&self, name: &str) -> Result<Unit, SysdError> {
        let s = self.state.lock().await;
        s.units
            .get(name)
            .cloned()
            .ok_or_else(|| SysdError::NotFound(format!("Unit {} not found.", name)))
    }
}

/// 转换 db 错误.
fn db_err(e: crate::db::DbError) -> SysdError {
    SysdError::Failed(e.to_string())
}

/// 计算 unit 的 active/sub 状态.
fn active_sub(p: Option<&Proc>) -> (ActiveState, SubState) {
    match p {
        Some(p) if p.failed => (ActiveState::Failed, SubState::Failed),
        Some(p) if p.active && p.child.is_some() => (ActiveState::Active, SubState::Running),
        Some(p) if p.active => (ActiveState::Active, SubState::Exited),
        _ => (ActiveState::Inactive, SubState::Dead),
    }
}

/// 启动一个 subprocess, 并接管其 stdout/stderr 写入 unit 日志.
///
/// 返回 tokio `Child` (其 stdout/stderr 已交由后台任务按行读取).
fn spawn_with_log<P: Pa>(
    pa: &P,
    argv: &[String],
    cwd: Option<&Path>,
    env: &[(String, String)],
    opts: pa::SpawnOptions,
    name: &str,
    logd: &Logd,
) -> Result<Child, SysdError> {
    let mut child = pa
        .spawn(argv, cwd, env, opts)
        .map_err(|e| SysdError::Failed(e.to_string()))?;

    if let Some(out) = child.stdout.take() {
        spawn_log_reader(name.to_string(), "stdout".to_string(), out, logd.clone());
    }
    if let Some(err) = child.stderr.take() {
        spawn_log_reader(name.to_string(), "stderr".to_string(), err, logd.clone());
    }
    Ok(child)
}

/// 启动 一个 service 的 主进程 (ExecStart).
fn spawn_child<P: Pa>(pa: &P, unit: &Unit, logd: &Logd) -> Result<Child, SysdError> {
    let argv = pa::split_cmd(&unit.service.exec_start);
    if argv.is_empty() {
        return Err(SysdError::Failed(format!(
            "unit `{}` ExecStart is empty",
            unit.name
        )));
    }
    let cwd = unit.service.working_directory.as_deref().map(Path::new);
    let child = spawn_with_log(
        pa,
        &argv,
        cwd,
        &unit.service.environment,
        pa::SpawnOptions {
            create_no_window: unit.win.create_no_window,
        },
        &unit.name,
        logd,
    )?;
    logd.write_with(
        "info",
        Some(&unit.name),
        &format!("started (pid {})", child.id().unwrap_or(0)),
    )
    .ok();
    Ok(child)
}

/// 运行 一个 Exec* 命令 (ExecReload / ExecStop): 启动 subprocess 并等待其退出.
async fn run_exec<P: Pa>(
    pa: &P,
    cmd: &str,
    unit: &Unit,
    name: &str,
    logd: &Logd,
) -> Result<(), SysdError> {
    let argv = pa::split_cmd(cmd);
    if argv.is_empty() {
        return Err(SysdError::Failed(format!(
            "unit `{}` exec command is empty",
            name
        )));
    }
    logd.write_with("info", Some(name), &format!("exec started `{}`", cmd))
        .ok();
    let cwd = unit.service.working_directory.as_deref().map(Path::new);
    let mut child = spawn_with_log(
        pa,
        &argv,
        cwd,
        &unit.service.environment,
        pa::SpawnOptions {
            create_no_window: unit.win.create_no_window,
        },
        name,
        logd,
    )?;
    let status = child
        .wait()
        .await
        .map_err(|e| SysdError::Failed(format!("exec `{}`: {}", name, e)))?;
    if status.success() {
        logd.write_with("info", Some(name), "exec exited (status 0)")
            .ok();
        Ok(())
    } else {
        logd.write_with("error", Some(name), &format!("exec failed ({})", status))
            .ok();
        Err(SysdError::Failed(format!(
            "exec `{}` failed: {}",
            name, status
        )))
    }
}

/// 启动 后台任务: 按行读取 子进程的 stdout/stderr, 写入对应 unit 日志.
///
/// `level` 分别为 `stdout` / `stderr`.
fn spawn_log_reader<R>(name: String, level: String, reader: R, logd: Logd)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                // 读到 EOF: 管道关闭 (子进程 退出), 结束本任务.
                Ok(0) => break,
                Ok(_) => {
                    // 去掉 行尾 换行符 (Windows `\r\n` / Unix `\n`).
                    while line.ends_with('\n') || line.ends_with('\r') {
                        line.pop();
                    }
                    if let Err(e) = logd.write_with(&level, Some(&name), &line) {
                        log::warn!("log `{}` {}: {}", name, level, e);
                    }
                }
                Err(e) => {
                    log::warn!("read `{}` {}: {}", name, level, e);
                    break;
                }
            }
        }
    });
}

/// 后台监控 一次 tick: 检测退出, 应用重启策略.
async fn monitor_tick<P: Pa>(state: &Arc<Mutex<State>>, logd: &Logd, pa: &Arc<P>) {
    let mut s = state.lock().await;
    let now = Instant::now();
    let names: Vec<String> = s.procs.keys().cloned().collect();

    for name in names {
        let unit = s.units.get(&name).cloned();
        let mut respawn = false;
        let mut log_exit = false;
        let mut exit_code = None;

        if let Some(p) = s.procs.get_mut(&name) {
            if let Some(child) = p.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let success = status.success();
                        let code = status.code();
                        p.child = None;
                        p.exit_status = code;
                        exit_code = code;
                        log_exit = true;

                        let (st, remain, restart) = unit
                            .as_ref()
                            .map(|u| {
                                (
                                    u.service.service_type,
                                    u.service.remain_after_exit,
                                    u.service.restart,
                                )
                            })
                            .unwrap_or((ServiceType::Simple, false, RestartPolicy::No));

                        match st {
                            ServiceType::Oneshot => {
                                p.active = false;
                            }
                            ServiceType::Simple => {
                                if !(success && remain) {
                                    p.active = false;
                                }
                            }
                        }

                        let should_restart = !p.stop_requested
                            && match restart {
                                RestartPolicy::Always => true,
                                RestartPolicy::OnFailure => !success,
                                RestartPolicy::No => false,
                            };

                        if should_restart {
                            p.failed = false;
                            p.restart_count += 1;
                            let sec = unit.as_ref().map(|u| u.service.restart_sec).unwrap_or(1);
                            p.next_restart = Some(now + Duration::from_secs(sec as u64));
                        } else {
                            p.failed = !success;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        logd.write_with("error", Some(&name), &format!("try_wait: {}", e))
                            .ok();
                    }
                }
            }

            // 到期 自动重启.
            if p.child.is_none() && !p.stop_requested {
                if let Some(t) = p.next_restart {
                    if now >= t {
                        p.next_restart = None;
                        respawn = true;
                    }
                }
            }
        }

        if log_exit {
            logd.write_with(
                "info",
                Some(&name),
                &format!("exited (status {:?})", exit_code),
            )
            .ok();
        }

        if respawn {
            if let Some(u) = &unit {
                match spawn_child(pa.as_ref(), u, logd) {
                    Ok(child) => {
                        if let Some(p) = s.procs.get_mut(&name) {
                            p.child = Some(child);
                            p.active = true;
                            p.failed = false;
                        }
                    }
                    Err(e) => {
                        logd.write_with("error", Some(&name), &format!("restart: {}", e))
                            .ok();
                        if let Some(p) = s.procs.get_mut(&name) {
                            p.failed = true;
                        }
                    }
                }
            }
        }
    }
}
