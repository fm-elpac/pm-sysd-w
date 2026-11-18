//! ServiceS 状态机 (纯计算).
//!
//! [`Sm`] 持有单个 service 的全部动态状态与业务逻辑. ServiceS 把 [`super::ServiceSM`]
//! 消息与内部事件 (子进程退出 / 定时器到期 / Exec* 完成) 统一喂给状态机, 再逐条执行
//! 状态机返回的 [`SmOutput`] 动作. 状态机不接触 oneshot / 进程 / 日志, 也不暴露状态
//! 读取接口; 对外只有 `new()` 与 `t()`.
//!
//! ## 状态使用说明
//!
//! 本状态机使用 [`ServiceSta`] 枚举追踪 service 状态. 当前功能集实际使用的状态:
//!
//! - `Dead`: 初始状态, 以及停止 / 自然退出 (未自动重启) 后的终态.
//! - `Start`: 启动阶段; 对 `Type=oneshot` 同时也是 ExecStart 进程的运行阶段
//!   (systemd 的 oneshot 没有独立的 `Running` 状态).
//! - `Running`: `Type=simple` 的主进程运行阶段.
//! - `Exited`: `RemainAfterExit=yes` 且进程成功退出后的 active 状态.
//! - `Reload`: 正在执行 `ExecReload`.
//! - `Stop`: 正在执行 `ExecStop`.
//! - `StopSigterm`: 正在终止 ExecStart 主进程. 本程序只有一种强制终止 (无
//!   SIGTERM/SIGKILL 区分), 故用该状态表示唯一的终止阶段.
//! - `Failed`: 启动失败 / 进程异常退出且未自动重启的失败状态.
//! - `AutoRestartQueued`: 已决定自动重启, 等待 `RestartSec` 定时器.
//!
//! 未使用的状态及原因:
//!
//! - `Condition`, `StartPre`, `StartPost`: 当前配置不支持条件检查与
//!   `ExecStartPre` / `ExecStartPost`.
//! - `RunningRevalidating`, `RefreshExtensions`, `RefreshCredentials`,
//!   `ReloadSignal`, `ReloadNotify`, `ReloadPost`, `Mounting`: 依赖 D-Bus /
//!   namespace / 信号通知等本程序未实现的功能.
//! - `StopWatchdog`, `StopPost`, `FinalWatchdog`, `FinalSigterm`, `FinalSigkill`:
//!   当前不支持 watchdog, 也没有 `ExecStopPost` / 多阶段终止.
//! - `DeadBeforeAutoRestart`, `FailedBeforeAutoRestart`, `AutoRestart`: 当前
//!   自动重启直接由 `AutoRestartQueued` + `begin_start()` 完成, 没有为这两个
//!   中间态保留可观察窗口.
//! - `DeadResourcesPinned`, `Cleaning`: 不支持资源钉扎与 `clean` 命令.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use serde::{Serialize, Serializer, ser::SerializeStruct};

use log::trace;

use crate::{
    conf::{RestartPolicy, ServiceType, Unit},
    logd::{instant_iso, now_millis},
    sysd::{SysdError, service_d::ServiceOp},
    sysu::{ServiceSta, UnitSta},
};

mod io;

pub use io::{ExitInfo, ServiceStatus, ServiceStatusInfo, SmInput, SmOutput};

/// ServiceS 状态机 (所有动态状态都封装在这里, 由 ServiceS 单线程访问).
///
/// 对外只有 [`Sm::new`] 与 [`Sm::t`] 两个方法; 状态读取通过 `SmInput` 查询事件
/// 与对应的 `SmOutput` 响应动作完成.
#[derive(Debug, Clone)]
pub struct Sm {
    /// service 配置快照 (整个生命周期保持不变)
    conf: Unit,
    /// 当前 service 状态
    state: ServiceSta,
    /// 是否有运行中的子进程
    child_running: bool,
    /// 运行中子进程的 pid
    child_pid: Option<u32>,
    /// 是否已请求停止 (停止后不再自动重启)
    stop_requested: bool,
    /// 自动重启次数
    restart_count: u32,
    /// 是否已安排定时重启
    restart_scheduled: bool,
    /// 是否停止后需要重新启动 (restart)
    restart_after_stop: bool,
    /// 停止模式
    stop_mode: StopMode,
    /// 当前停止序列中是否已经执行过 ExecStop
    exec_stop_done: bool,
    /// 启动超时是否已触发 (等待子进程退出后转为 failed)
    start_timeout_hit: bool,
    /// 最后一次退出状态码
    exit_status: Option<i32>,
    /// 当前状态进入时间 (Unix 毫秒时间戳). 初始 `Dead` 状态无时间.
    state_since: Option<i64>,
    /// 是否有命令正在等待异步完成 (此时拒绝新的修改命令)
    busy: bool,
    /// 启动尝试时间戳 (用于 StartLimitIntervalSec / StartLimitBurst)
    start_attempts: VecDeque<Instant>,
}

impl Serialize for Sm {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("Sm", 15)?;
        s.serialize_field("conf", &self.conf)?;
        s.serialize_field("state", &self.state)?;
        s.serialize_field("child_running", &self.child_running)?;
        s.serialize_field("child_pid", &self.child_pid)?;
        s.serialize_field("stop_requested", &self.stop_requested)?;
        s.serialize_field("restart_count", &self.restart_count)?;
        s.serialize_field("restart_scheduled", &self.restart_scheduled)?;
        s.serialize_field("restart_after_stop", &self.restart_after_stop)?;
        s.serialize_field("stop_mode", &self.stop_mode)?;
        s.serialize_field("exec_stop_done", &self.exec_stop_done)?;
        s.serialize_field("start_timeout_hit", &self.start_timeout_hit)?;
        s.serialize_field("exit_status", &self.exit_status)?;
        s.serialize_field("state_since", &self.state_since)?;
        s.serialize_field("busy", &self.busy)?;
        let start_attempts: Vec<String> = self.start_attempts.iter().map(instant_iso).collect();
        s.serialize_field("start_attempts", &start_attempts)?;
        s.end()
    }
}

/// 停止模式 (影响是否执行 ExecStop)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
enum StopMode {
    /// 未停止
    None,
    /// 优雅停止 (执行 ExecStop)
    Stop,
    /// 强制终止 (不执行 ExecStop)
    Kill,
}

impl Sm {
    /// 用 service 配置快照创建
    pub fn new(conf: Unit) -> Self {
        Self {
            conf,
            state: ServiceSta::Dead,
            child_running: false,
            child_pid: None,
            stop_requested: false,
            restart_count: 0,
            restart_scheduled: false,
            restart_after_stop: false,
            stop_mode: StopMode::None,
            exec_stop_done: false,
            start_timeout_hit: false,
            exit_status: None,
            state_since: None,
            busy: false,
            start_attempts: VecDeque::new(),
        }
    }

    /// 状态机主要处理逻辑 (入口)
    pub fn t(self, input: SmInput) -> (Self, Vec<SmOutput>) {
        trace!("{} in {:?}", self.name(), input);
        let (s, out) = self.it(input);
        trace!("{} out {:?}", s.name(), out);
        (s, out)
    }

    /// unit 名称
    fn name(&self) -> &str {
        &self.conf.name
    }

    /// 设置当前 service 状态, 并在状态变化时记录进入时间.
    fn set_state(&mut self, new: ServiceSta) {
        if self.state != new {
            self.state = new;
            self.state_since = Some(now_millis());
        }
    }

    /// 状态机主要处理逻辑 (内部实现, 由 [`Sm::t`] 包装 trace)
    fn it(mut self, input: SmInput) -> (Self, Vec<SmOutput>) {
        let actions = match input {
            SmInput::Cmd(op) => self.handle_cmd(op),
            SmInput::QueryState => vec![SmOutput::ReplyState(self.active_sub())],
            SmInput::QueryStatus => vec![SmOutput::ReplyStatus(self.query_status())],
            SmInput::Spawned { pid } => self.handle_spawned(pid),
            SmInput::SpawnFailed(e) => self.handle_spawn_failed(e),
            SmInput::ChildExited(info) => self.handle_child_exited(info),
            SmInput::ExecStopDone => self.handle_exec_stop_done(),
            SmInput::ReloadDone(result) => self.handle_reload_done(result),
            SmInput::RestartTimer => self.handle_restart_timer(),
            SmInput::StartTimeout => self.handle_start_timeout(),
        };
        (self, actions)
    }

    /// 处理外部命令 (复用 ServiceOp)
    fn handle_cmd(&mut self, op: ServiceOp) -> Vec<SmOutput> {
        // 已有命令等待异步完成时, 拒绝新的修改命令; 查询不受影响.
        if self.busy {
            return vec![res_err("service busy")];
        }

        match op {
            ServiceOp::Start => self.handle_start(),
            ServiceOp::Stop => self.handle_stop(StopMode::Stop),
            ServiceOp::Kill => self.handle_stop(StopMode::Kill),
            ServiceOp::Restart => self.handle_restart(),
            ServiceOp::TryRestart => {
                if self.service_active() {
                    self.handle_restart()
                } else {
                    vec![SmOutput::ReplyCmd(Ok(()))]
                }
            }
            ServiceOp::Reload => {
                // TODO: sysd 暂只支持一个 ExecReload, 多行语法只取第一行.
                if self.conf.service.exec_reload.is_empty() {
                    return vec![res_err(format!(
                        "Unit {} has no ExecReload.",
                        self.conf.name
                    ))];
                }
                self.busy = true;
                self.set_state(ServiceSta::Reload);
                vec![SmOutput::RunExecReload]
            }
            ServiceOp::ReloadOrRestart => {
                if !self.conf.service.exec_reload.is_empty() {
                    self.busy = true;
                    self.set_state(ServiceSta::Reload);
                    vec![SmOutput::RunExecReload]
                } else {
                    self.handle_restart()
                }
            }
            ServiceOp::TryReloadOrRestart => {
                if !self.service_active() {
                    vec![SmOutput::ReplyCmd(Ok(()))]
                } else if !self.conf.service.exec_reload.is_empty() {
                    self.busy = true;
                    self.set_state(ServiceSta::Reload);
                    vec![SmOutput::RunExecReload]
                } else {
                    self.handle_restart()
                }
            }
            ServiceOp::ResetFailed => {
                self.reset_failed();
                vec![SmOutput::ReplyCmd(Ok(()))]
            }
        }
    }

    /// 当前 active / sub 状态
    fn active_sub(&self) -> (UnitSta, ServiceSta) {
        let active = match self.state {
            ServiceSta::Dead => UnitSta::Inactive,
            ServiceSta::Start => UnitSta::Activating,
            ServiceSta::Running => UnitSta::Active,
            ServiceSta::Exited => UnitSta::Active,
            ServiceSta::Reload => UnitSta::Reloading,
            ServiceSta::Stop => UnitSta::Deactivating,
            ServiceSta::StopSigterm => UnitSta::Deactivating,
            ServiceSta::Failed => UnitSta::Failed,
            ServiceSta::AutoRestartQueued => UnitSta::Activating,
            // 其余状态当前状态机不会进入.
            other => UnitSta::from(other),
        };
        (active, self.state)
    }

    /// 当前详细状态快照
    fn query_status(&self) -> ServiceStatusInfo {
        let (active, sub) = self.active_sub();
        ServiceStatusInfo {
            active,
            sub,
            pid: self.child_pid,
            restart_count: self.restart_count,
            exit_status: self.exit_status,
            since: self.state_since,
        }
    }

    /// 是否 failed
    fn failed(&self) -> bool {
        matches!(self.state, ServiceSta::Failed)
    }

    /// 当前终止状态快照
    fn status(&self) -> ServiceStatus {
        ServiceStatus {
            failed: self.failed(),
            restart_count: self.restart_count,
            exit_status: self.exit_status,
            since: self.state_since,
        }
    }

    /// 构造退出请求动作 (上报状态, 由 ServiceD 回发 Exit)
    fn request_exit(&mut self) -> Vec<SmOutput> {
        vec![SmOutput::RequestExit {
            status: self.status(),
        }]
    }

    /// 是否 active (Run/Exited)
    fn service_active(&self) -> bool {
        matches!(self.state, ServiceSta::Running | ServiceSta::Exited)
    }

    /// 是否达到启动限流
    fn start_limit_hit(&self) -> bool {
        let interval = self.conf.start_limit_interval_sec;
        let burst = self.conf.start_limit_burst;
        if interval == 0 || burst == 0 {
            return false;
        }
        let cutoff = Instant::now() - Duration::from_secs(interval as u64);
        self.start_attempts.iter().filter(|t| **t >= cutoff).count() >= burst as usize
    }

    /// 记录一次启动尝试 (用于启动限流)
    fn record_start_attempt(&mut self) {
        let interval = self.conf.start_limit_interval_sec;
        if interval == 0 {
            return;
        }
        let cutoff = Instant::now() - Duration::from_secs(interval as u64);
        while self.start_attempts.front().is_some_and(|t| *t < cutoff) {
            self.start_attempts.pop_front();
        }
        self.start_attempts.push_back(Instant::now());
    }

    /// 重置 failed 状态与启动限流计数
    fn reset_failed(&mut self) {
        self.start_attempts.clear();
        if matches!(self.state, ServiceSta::Failed) {
            self.set_state(ServiceSta::Dead);
        }
    }

    /// 开始启动 (设置状态并返回 spawn 动作).
    ///
    /// 不负责记录启动限流, 也不重置自动重启次数与 busy; 由调用方处理.
    fn begin_start(&mut self) -> Vec<SmOutput> {
        // TODO: conf 允许 ExecStart 为空 (配合 RemainAfterExit=yes), 此时应直接
        // 进入 Exited; sysd 暂不支持, 仍按普通启动处理.
        self.set_state(match self.conf.service.service_type {
            ServiceType::Simple => ServiceSta::Running,
            ServiceType::Oneshot => ServiceSta::Start,
        });
        self.child_running = true;
        self.child_pid = None;
        self.exit_status = None;
        self.stop_requested = false;
        self.restart_after_stop = false;
        self.stop_mode = StopMode::None;
        self.exec_stop_done = false;
        self.start_timeout_hit = false;
        self.restart_scheduled = false;

        let mut actions = vec![SmOutput::SpawnChild];
        if self.conf.service.service_type == ServiceType::Oneshot
            && self.conf.service.timeout_start_sec > 0
        {
            actions.insert(
                0,
                SmOutput::ScheduleStartTimeout {
                    secs: self.conf.service.timeout_start_sec,
                },
            );
        }
        actions
    }

    /// Start 命令
    fn handle_start(&mut self) -> Vec<SmOutput> {
        if self.child_running || self.service_active() || matches!(self.state, ServiceSta::Start) {
            return vec![SmOutput::ReplyCmd(Ok(()))];
        }
        if self.start_limit_hit() {
            return vec![res_err("start-limit-hit")];
        }

        let mut actions = Vec::new();
        if self.restart_scheduled {
            actions.push(SmOutput::CancelRestart);
            self.restart_scheduled = false;
        }
        self.record_start_attempt();
        self.restart_count = 0;
        self.busy = true;
        actions.extend(self.begin_start());
        actions
    }

    /// Stop / Kill 命令
    fn handle_stop(&mut self, mode: StopMode) -> Vec<SmOutput> {
        self.stop_requested = true;
        self.stop_mode = mode;
        self.restart_after_stop = false;
        self.exec_stop_done = false;

        let cancel_start = matches!(self.state, ServiceSta::Start);
        let cancel_restart = self.restart_scheduled;
        self.restart_scheduled = false;
        self.set_state(ServiceSta::Stop);

        let mut actions = Vec::new();
        if cancel_restart {
            actions.push(SmOutput::CancelRestart);
        }
        if cancel_start {
            actions.push(SmOutput::CancelStartTimeout);
        }

        // TODO: sysd 暂只支持一个 ExecStop, 多行语法只取第一行.
        if self.child_running {
            if mode == StopMode::Stop && self.conf.service.exec_stop.first().is_some() {
                self.busy = true;
                actions.push(SmOutput::RunExecStop);
            } else {
                self.busy = true;
                self.set_state(ServiceSta::StopSigterm);
                actions.push(SmOutput::KillChild);
            }
        } else if self.service_active() {
            // RemainAfterExit=yes 且已退出: 无子进程可终止.
            if mode == StopMode::Stop && self.conf.service.exec_stop.first().is_some() {
                self.busy = true;
                actions.push(SmOutput::RunExecStop);
            } else {
                self.set_state(ServiceSta::Dead);
                actions.push(SmOutput::ReplyCmd(Ok(())));
                actions.extend(self.request_exit());
            }
        } else {
            // 已 inactive (例如等待定时重启): 取消重启并终止
            self.set_state(ServiceSta::Dead);
            actions.push(SmOutput::ReplyCmd(Ok(())));
            actions.extend(self.request_exit());
        }
        actions
    }

    /// Restart 命令 (先停止, 再启动)
    fn handle_restart(&mut self) -> Vec<SmOutput> {
        if self.child_running || self.service_active() {
            if self.start_limit_hit() {
                return vec![res_err("start-limit-hit")];
            }
            self.record_start_attempt();
            self.restart_count = 0;
            self.busy = true;
            self.restart_stop_phase()
        } else {
            self.handle_start()
        }
    }

    /// Restart 的停止阶段
    fn restart_stop_phase(&mut self) -> Vec<SmOutput> {
        self.stop_requested = true;
        self.restart_after_stop = true;
        self.stop_mode = StopMode::Stop;
        self.restart_scheduled = false;
        self.exec_stop_done = false;

        let cancel_start = matches!(self.state, ServiceSta::Start);
        self.set_state(ServiceSta::Stop);

        let mut actions = Vec::new();
        if cancel_start {
            actions.push(SmOutput::CancelStartTimeout);
        }

        // TODO: sysd 暂只支持一个 ExecStop, 多行语法只取第一行.
        if self.child_running {
            if self.conf.service.exec_stop.first().is_some() {
                actions.push(SmOutput::RunExecStop);
            } else {
                actions.push(SmOutput::KillChild);
            }
        } else if self.conf.service.exec_stop.first().is_some() {
            // RemainAfterExit=yes 且已退出: 先执行 ExecStop, 再直接重新启动.
            actions.push(SmOutput::RunExecStop);
        } else {
            // 无 ExecStop: 直接进入启动阶段.
            self.restart_after_stop = false;
            self.stop_requested = false;
            self.stop_mode = StopMode::None;
            actions.extend(self.begin_start());
        }
        actions
    }

    /// 子进程已创建
    fn handle_spawned(&mut self, pid: Option<u32>) -> Vec<SmOutput> {
        self.child_pid = pid;
        // simple 类型: 启动即命令完成; oneshot 等待 ExecStart (child exit).
        if matches!(self.state, ServiceSta::Running) && self.busy {
            self.busy = false;
            vec![SmOutput::ReplyCmd(Ok(()))]
        } else {
            vec![]
        }
    }

    /// 子进程创建失败
    fn handle_spawn_failed(&mut self, e: SysdError) -> Vec<SmOutput> {
        self.child_running = false;
        self.set_state(ServiceSta::Failed);
        self.restart_scheduled = false;
        self.start_timeout_hit = false;

        let mut actions = Vec::new();
        if self.busy {
            actions.push(SmOutput::ReplyCmd(Err(e)));
            self.busy = false;
        }
        actions.extend(self.request_exit());
        actions
    }

    /// ExecStop 完成后的后续动作
    fn handle_exec_stop_done(&mut self) -> Vec<SmOutput> {
        self.exec_stop_done = true;
        if self.restart_after_stop {
            if self.child_running {
                self.set_state(ServiceSta::StopSigterm);
                vec![SmOutput::KillChild]
            } else {
                self.restart_after_stop = false;
                self.stop_requested = false;
                self.stop_mode = StopMode::None;
                self.begin_start()
            }
        } else if self.child_running {
            self.set_state(ServiceSta::StopSigterm);
            vec![SmOutput::KillChild]
        } else {
            self.set_state(ServiceSta::Dead);
            let mut actions = Vec::new();
            if self.busy {
                actions.push(SmOutput::ReplyCmd(Ok(())));
                self.busy = false;
            }
            actions.extend(self.request_exit());
            actions
        }
    }

    /// ExecReload 完成后的状态恢复
    fn handle_reload_done(&mut self, result: Result<(), SysdError>) -> Vec<SmOutput> {
        self.set_state(if self.child_running {
            ServiceSta::Running
        } else {
            ServiceSta::Exited
        });

        let mut actions = Vec::new();
        if self.busy {
            actions.push(SmOutput::ReplyCmd(result));
            self.busy = false;
        }
        actions
    }

    /// 子进程退出
    fn handle_child_exited(&mut self, info: ExitInfo) -> Vec<SmOutput> {
        self.child_running = false;
        self.child_pid = None;
        self.exit_status = info.code;

        if self.start_timeout_hit {
            self.start_timeout_hit = false;
            self.set_state(ServiceSta::Failed);
            let mut actions = Vec::new();
            if self.busy {
                actions.push(SmOutput::ReplyCmd(Err(SysdError::Failed(
                    "service failed".to_string(),
                ))));
                self.busy = false;
            }
            actions.extend(self.request_exit());
            return actions;
        }

        if self.restart_after_stop {
            self.restart_after_stop = false;
            self.stop_requested = false;
            self.stop_mode = StopMode::None;
            self.exec_stop_done = false;
            self.start_timeout_hit = false;
            return self.begin_start();
        }

        if self.stop_requested {
            self.set_state(ServiceSta::Dead);
            let mut actions = Vec::new();
            if self.busy {
                actions.push(SmOutput::ReplyCmd(Ok(())));
                self.busy = false;
            }
            actions.extend(self.request_exit());
            return actions;
        }

        // 自然退出: 应用 service 类型与 RemainAfterExit 语义
        let remain = self.conf.service.remain_after_exit;
        self.set_state(if info.success && remain {
            ServiceSta::Exited
        } else {
            ServiceSta::Dead
        });

        let should_restart = match self.conf.service.restart {
            RestartPolicy::Always => true,
            RestartPolicy::OnFailure => !info.success,
            RestartPolicy::No => false,
        };

        if should_restart {
            if self.start_limit_hit() {
                self.set_state(ServiceSta::Failed);
                let mut actions = Vec::new();
                if self.busy {
                    actions.push(SmOutput::ReplyCmd(Err(SysdError::Failed(
                        "service failed".to_string(),
                    ))));
                    self.busy = false;
                }
                actions.extend(self.request_exit());
                return actions;
            }
            self.record_start_attempt();
            self.set_state(ServiceSta::AutoRestartQueued);
            self.restart_count += 1;
            self.restart_scheduled = true;

            let mut actions = Vec::new();
            if self.busy {
                // oneshot Start 命令在此完成 (自动重启已排队).
                let result = if info.success {
                    Ok(())
                } else {
                    Err(SysdError::Failed(format!(
                        "ExecStart exited with status {}",
                        info.code.unwrap_or(1)
                    )))
                };
                actions.push(SmOutput::ReplyCmd(result));
                self.busy = false;
            }
            actions.push(SmOutput::ScheduleRestart {
                secs: self.conf.service.restart_sec,
            });
            return actions;
        }

        if matches!(self.state, ServiceSta::Exited) {
            let mut actions = Vec::new();
            if self.busy {
                actions.push(SmOutput::ReplyCmd(Ok(())));
                self.busy = false;
            }
            return actions;
        }

        if !info.success {
            self.set_state(ServiceSta::Failed);
        }
        let mut actions = Vec::new();
        if self.busy {
            let result = if self.failed() {
                Err(SysdError::Failed("service failed".to_string()))
            } else {
                Ok(())
            };
            actions.push(SmOutput::ReplyCmd(result));
            self.busy = false;
        }
        actions.extend(self.request_exit());
        actions
    }

    /// 重启定时器到期
    fn handle_restart_timer(&mut self) -> Vec<SmOutput> {
        self.restart_scheduled = false;
        if self.stop_requested {
            return vec![];
        }
        self.begin_start()
    }

    /// 启动超时 (仅 Type=oneshot)
    fn handle_start_timeout(&mut self) -> Vec<SmOutput> {
        if self.child_running && matches!(self.state, ServiceSta::Start) {
            self.start_timeout_hit = true;
            self.set_state(ServiceSta::StopSigterm);
            vec![SmOutput::KillChild]
        } else {
            self.start_timeout_hit = false;
            self.set_state(ServiceSta::Failed);
            let mut actions = Vec::new();
            if self.busy {
                actions.push(SmOutput::ReplyCmd(Err(SysdError::Failed(
                    "service failed".to_string(),
                ))));
                self.busy = false;
            }
            actions.extend(self.request_exit());
            actions
        }
    }
}

/// 构造命令错误响应动作
fn res_err(msg: impl Into<String>) -> SmOutput {
    SmOutput::ReplyCmd(Err(SysdError::Failed(msg.into())))
}

#[cfg(test)]
mod test;
