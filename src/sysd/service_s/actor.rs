//! ServiceS 状态机 (仅计算/决策, 不执行操作).
//!
//! ServiceSSM 持有单个 service 的动态状态. ServiceS 把 [`super::ServiceSM`] 消息与
//! 内部事件喂给状态机, 再执行状态机返回的 [`Action`] 列表.
//! 状态机不接触 oneshot / 进程 / 日志, 便于测试.

use crate::{
    conf::{RestartPolicy, ServiceType, Unit},
    sysd::{ServiceSta, UnitSta},
};

use super::super::service_d::ServiceOp;

/// 子进程退出信息 (纯数据)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitInfo {
    /// 是否成功退出 (退出码 0)
    pub success: bool,
    /// 退出状态码
    pub code: Option<i32>,
}

/// 一个 service 的终止状态 (上报给 ServiceD, 用于保留最近状态)
#[derive(Debug, Clone, Copy)]
pub struct ServiceStatus {
    /// 是否 failed
    pub failed: bool,
    /// 自动重启次数
    pub restart_count: u32,
    /// 最后一次退出状态码
    pub exit_status: Option<i32>,
}

impl ServiceStatus {
    /// 由终止状态推导 active / sub 状态
    pub fn active_sub(&self) -> (UnitSta, ServiceSta) {
        if self.failed {
            (UnitSta::Failed, ServiceSta::Failed)
        } else {
            (UnitSta::Inactive, ServiceSta::Dead)
        }
    }
}

/// 停止模式 (影响是否执行 ExecStop)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopMode {
    /// 未停止
    None,
    /// 优雅停止 (执行 ExecStop)
    Stop,
    /// 强制终止 (不执行 ExecStop)
    Kill,
}

/// 事件 (输入)
pub enum Event {
    /// 外部命令 (复用 ServiceOp)
    Cmd(ServiceOp),
    /// 子进程已退出 (自然退出或强制终止)
    ChildExited(ExitInfo),
    /// 重启定时器到期
    RestartTimer,
    /// 子进程启动失败
    SpawnFailed,
}

/// 动作 (输出)
pub enum Action {
    /// 启动子进程
    SpawnChild,
    /// 终止子进程并等待其退出
    KillChild,
    /// 执行 ExecStop
    RunExecStop,
    /// 执行 ExecReload
    RunExecReload,
    /// 安排定时重启
    ScheduleRestart { secs: u32 },
    /// 取消定时重启
    CancelRestart,
    /// 上报终止状态 (然后 ServiceS 退出)
    NotifyTerminated { status: ServiceStatus },
}

/// ServiceS 状态机 (所有动态状态都封装在这里, 由 ServiceS 单线程访问)
pub struct ServiceSSM {
    /// service 配置快照 (整个生命周期保持不变)
    conf: Unit,
    /// 是否视为 active
    active: bool,
    /// 是否 failed
    failed: bool,
    /// 是否已请求停止 (停止后不再自动重启)
    stop_requested: bool,
    /// 自动重启次数
    restart_count: u32,
    /// 是否有运行中的子进程
    child_running: bool,
    /// 运行中子进程的 pid
    child_pid: Option<u32>,
    /// 是否已安排定时重启
    restart_scheduled: bool,
    /// 是否停止后需要重新启动 (restart)
    restart_after_stop: bool,
    /// 停止模式
    stop_mode: StopMode,
    /// 最后一次退出状态码
    exit_status: Option<i32>,
    /// 是否已终止 (run 循环结束)
    terminated: bool,
}

impl ServiceSSM {
    /// 用 service 配置快照创建
    pub fn new(conf: Unit) -> Self {
        Self {
            conf,
            active: false,
            failed: false,
            stop_requested: false,
            restart_count: 0,
            child_running: false,
            child_pid: None,
            restart_scheduled: false,
            restart_after_stop: false,
            stop_mode: StopMode::None,
            exit_status: None,
            terminated: false,
        }
    }

    /// 读取配置快照
    pub fn conf(&self) -> &Unit {
        &self.conf
    }

    /// 当前 active / sub 状态
    pub fn active_sub(&self) -> (UnitSta, ServiceSta) {
        if self.failed {
            (UnitSta::Failed, ServiceSta::Failed)
        } else if self.active && self.child_running {
            (UnitSta::Active, ServiceSta::Running)
        } else if self.active {
            (UnitSta::Active, ServiceSta::Exited)
        } else {
            (UnitSta::Inactive, ServiceSta::Dead)
        }
    }

    /// 自动重启次数
    pub fn restart_count(&self) -> u32 {
        self.restart_count
    }

    /// 最后一次退出状态码
    pub fn exit_status(&self) -> Option<i32> {
        self.exit_status
    }

    /// 运行中子进程的 pid
    pub fn child_pid(&self) -> Option<u32> {
        self.child_pid
    }

    /// 更新运行中子进程的 pid (由 ServiceS 执行 spawn / kill 时调用)
    pub fn set_child_pid(&mut self, pid: Option<u32>) {
        self.child_pid = pid;
    }

    /// 是否已终止
    pub fn terminated(&self) -> bool {
        self.terminated
    }

    /// 是否 active (测试用)
    #[cfg(test)]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// 是否 failed (测试用)
    #[cfg(test)]
    pub fn is_failed(&self) -> bool {
        self.failed
    }

    /// 是否有运行中的子进程 (测试用)
    #[cfg(test)]
    pub fn child_running(&self) -> bool {
        self.child_running
    }

    /// 是否已安排定时重启 (测试用)
    #[cfg(test)]
    pub fn restart_scheduled(&self) -> bool {
        self.restart_scheduled
    }

    /// 处理一个事件, 返回要执行的动作
    pub fn handle(&mut self, ev: Event) -> Result<Vec<Action>, String> {
        Ok(match ev {
            Event::Cmd(op) => self.cmd_actions(op)?,
            Event::ChildExited(info) => self.child_exited_actions(info),
            Event::RestartTimer => self.restart_timer_actions(),
            Event::SpawnFailed => self.spawn_failed_actions(),
        })
    }

    /// 处理外部命令 (复用 ServiceOp)
    fn cmd_actions(&mut self, op: ServiceOp) -> Result<Vec<Action>, String> {
        Ok(match op {
            ServiceOp::Start => self.start_actions(),
            ServiceOp::Stop => self.stop_actions(StopMode::Stop),
            ServiceOp::Kill => self.stop_actions(StopMode::Kill),
            ServiceOp::Restart => self.restart_actions(),
            ServiceOp::TryRestart => {
                if self.active {
                    self.restart_actions()
                } else {
                    vec![]
                }
            }
            ServiceOp::Reload => {
                if self.conf.service.exec_reload.is_none() {
                    return Err(format!("Unit {} has no ExecReload.", self.conf.name));
                }
                vec![Action::RunExecReload]
            }
            ServiceOp::ReloadOrRestart => {
                if self.conf.service.exec_reload.is_some() {
                    vec![Action::RunExecReload]
                } else {
                    self.restart_actions()
                }
            }
            ServiceOp::TryReloadOrRestart => {
                if self.active {
                    if self.conf.service.exec_reload.is_some() {
                        vec![Action::RunExecReload]
                    } else {
                        self.restart_actions()
                    }
                } else {
                    vec![]
                }
            }
            ServiceOp::ResetFailed => {
                self.failed = false;
                vec![]
            }
        })
    }

    /// 当前终止状态快照
    fn status(&self) -> ServiceStatus {
        ServiceStatus {
            failed: self.failed,
            restart_count: self.restart_count,
            exit_status: self.exit_status,
        }
    }

    /// 构造终止动作 (并标记已终止)
    fn notify_terminated(&mut self) -> Vec<Action> {
        self.terminated = true;
        vec![Action::NotifyTerminated {
            status: self.status(),
        }]
    }

    fn start_actions(&mut self) -> Vec<Action> {
        if self.active || self.child_running {
            // 已 active: 幂等成功
            return vec![];
        }
        self.active = true;
        self.failed = false;
        self.stop_requested = false;
        self.restart_count = 0;
        self.child_running = true;
        self.restart_scheduled = false;
        self.restart_after_stop = false;
        self.stop_mode = StopMode::None;
        self.exit_status = None;
        vec![Action::SpawnChild]
    }

    fn stop_actions(&mut self, mode: StopMode) -> Vec<Action> {
        self.stop_requested = true;
        self.stop_mode = mode;
        self.restart_scheduled = false;
        self.restart_after_stop = false;

        if self.child_running {
            self.active = false;
            vec![Action::KillChild]
        } else if self.active {
            // active 但无子进程 (RemainAfterExit=yes)
            self.active = false;
            if mode == StopMode::Stop {
                self.terminated = true;
                vec![
                    Action::RunExecStop,
                    Action::NotifyTerminated {
                        status: self.status(),
                    },
                ]
            } else {
                self.notify_terminated()
            }
        } else {
            // 已 inactive (例如等待定时重启): 取消重启并终止
            self.terminated = true;
            vec![
                Action::CancelRestart,
                Action::NotifyTerminated {
                    status: self.status(),
                },
            ]
        }
    }

    fn restart_actions(&mut self) -> Vec<Action> {
        if self.child_running || self.active {
            self.stop_requested = true;
            self.stop_mode = StopMode::Stop;
            self.active = false;
            self.restart_scheduled = false;
            if self.child_running {
                self.restart_after_stop = true;
                vec![Action::KillChild]
            } else {
                // active 但无子进程 (RemainAfterExit=yes): 直接重新启动
                self.stop_requested = false;
                self.stop_mode = StopMode::None;
                self.failed = false;
                self.restart_count = 0;
                self.exit_status = None;
                self.active = true;
                self.child_running = true;
                vec![Action::RunExecStop, Action::SpawnChild]
            }
        } else {
            self.start_actions()
        }
    }

    fn child_exited_actions(&mut self, info: ExitInfo) -> Vec<Action> {
        self.child_running = false;
        self.exit_status = info.code;

        if self.restart_after_stop {
            // restart 的停止阶段完成: 执行 ExecStop 并重新启动
            self.restart_after_stop = false;
            self.stop_requested = false;
            self.stop_mode = StopMode::None;
            self.failed = false;
            self.restart_count = 0;
            self.exit_status = None;
            self.active = true;
            self.child_running = true;
            return vec![Action::RunExecStop, Action::SpawnChild];
        }

        if self.stop_requested {
            self.terminated = true;
            let mut actions = Vec::new();
            if self.stop_mode == StopMode::Stop {
                actions.push(Action::RunExecStop);
            }
            actions.push(Action::NotifyTerminated {
                status: self.status(),
            });
            return actions;
        }

        // 自然退出: 应用 service 类型与重启策略
        let st = self.conf.service.service_type;
        let remain = self.conf.service.remain_after_exit;
        match st {
            ServiceType::Oneshot => {
                self.active = false;
            }
            ServiceType::Simple => {
                if !(info.success && remain) {
                    self.active = false;
                }
            }
        }

        let should_restart = match self.conf.service.restart {
            RestartPolicy::Always => true,
            RestartPolicy::OnFailure => !info.success,
            RestartPolicy::No => false,
        };

        if should_restart {
            self.failed = false;
            self.restart_count += 1;
            self.restart_scheduled = true;
            vec![Action::ScheduleRestart {
                secs: self.conf.service.restart_sec,
            }]
        } else {
            self.failed = !info.success;
            if self.active {
                // RemainAfterExit=yes 且成功: 保持 active
                vec![]
            } else {
                self.notify_terminated()
            }
        }
    }

    fn restart_timer_actions(&mut self) -> Vec<Action> {
        self.restart_scheduled = false;
        if self.stop_requested {
            return vec![];
        }
        self.active = true;
        self.failed = false;
        self.child_running = true;
        vec![Action::SpawnChild]
    }

    fn spawn_failed_actions(&mut self) -> Vec<Action> {
        self.active = false;
        self.failed = true;
        self.child_running = false;
        self.restart_scheduled = false;
        self.notify_terminated()
    }
}
