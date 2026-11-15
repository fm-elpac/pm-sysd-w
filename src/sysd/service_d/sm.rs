//! ServiceD 状态机 (纯计算).
//!
//! [`Sm`] 持有活跃 service 的配置 (`name -> Unit`), 已入队待启动的 name, 以及
//! 最近终止的 service 状态. ServiceD 把 [`super::ServiceDM`] 消息与内部事件
//! (job 队列回报 / ServiceS 上报 / 停止完成回报) 喂给状态机, 再执行状态机返回的
//! [`SmOutput`] 动作列表. 状态机不持有 ServiceS 句柄, 也不接触 logd / 消息发送
//! 通道; ServiceS 句柄由 ServiceD actor 持有.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use crate::{
    conf::Unit,
    jsonl::{SdReqUnitOp, SdRes},
    sysd::{SysdError, service_s::ServiceStatus},
};

use super::ServiceCmdUnit;

/// 转发给 ServiceS 的单个命令 (不含回执, 由 ServiceD 执行时注入回执)
#[derive(Clone, Copy)]
pub enum ServiceOp {
    Start,
    Stop,
    Kill,
    Restart,
    TryRestart,
    Reload,
    ReloadOrRestart,
    TryReloadOrRestart,
    ResetFailed,
}

/// 状态机输入数据 (事件)
pub enum SmInput {
    /// 单个 unit 操作
    Unit(ServiceCmdUnit),
    /// 批量启动 (default.target)
    StartMany(Vec<(String, Unit)>),
    /// 停止所有 service 并退出
    Exit,
    /// job 队列已创建 ServiceS (携带 name 与配置)
    ServiceStarted { name: String, conf: Unit },
    /// ServiceS 已终止 (携带状态)
    ServiceTerminated { name: String, status: ServiceStatus },
    /// 后台 task 已完成停止所有 service
    StopAllDone(Result<(), SysdError>),
}

/// 状态机输出的动作 (由 ServiceD actor 执行)
pub enum SmOutput {
    /// 立即返回 SdReq 响应 (Unit 命令路径)
    Reply(SdRes),
    /// 立即返回 StartMany / Exit 响应
    ReplyResult(Result<(), SysdError>),
    /// 转发命令给单个 ServiceS.
    ///
    /// Unit 命令路径等待回执并回复调用方; StartMany 路径不等待回执, 失败仅记录日志.
    SendServiceS { name: String, op: ServiceOp },
    /// 转发命令给全部 ServiceS (Unit 命令路径, 汇总回复)
    SendServiceSAll { op: ServiceOp },
    /// 入队 start job.
    ///
    /// Unit 命令路径等待回执并回复调用方; StartMany 路径不等待回执, 失败仅记录日志.
    EnqueueStart { name: String, conf: Unit },
    /// 停止所有 service (等待完成)
    StopAll,
    /// 退出 ServiceD 主循环
    Exit,
}

/// ServiceD 状态机
pub struct Sm {
    /// 活跃 service 的配置 (name -> conf). 对应一个已创建的 ServiceS.
    active: HashMap<String, Unit>,
    /// 已入队 start job 但尚未创建 ServiceS 的 name
    pending: HashSet<String>,
    /// 最近终止 service 的状态
    last: HashMap<String, ServiceStatus>,
    /// 手动 start 的启动尝试时间戳 (StartLimitIntervalSec / StartLimitBurst)
    start_attempts: HashMap<String, VecDeque<Instant>>,
    /// 是否正在退出 (等待所有 service 停止)
    stopping: bool,
}

impl Sm {
    /// 初始状态
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
            pending: HashSet::new(),
            last: HashMap::new(),
            start_attempts: HashMap::new(),
            stopping: false,
        }
    }

    /// 状态机主要处理逻辑 (入口)
    pub fn t(mut self, input: SmInput) -> (Self, Vec<SmOutput>) {
        let actions = match input {
            SmInput::Unit(cmd) => self.handle_unit(cmd),
            SmInput::StartMany(services) => self.handle_start_many(services),
            SmInput::Exit => self.handle_exit(),
            SmInput::ServiceStarted { name, conf } => self.handle_service_started(name, conf),
            SmInput::ServiceTerminated { name, status } => {
                self.handle_service_terminated(name, status)
            }
            SmInput::StopAllDone(result) => self.handle_stop_all_done(result),
        };
        (self, actions)
    }

    /// 读取运行中 service 的旧配置 (供查询)
    pub fn running_unit(&self, name: &str) -> Option<Unit> {
        self.active.get(name).cloned()
    }

    /// 读取最近终止状态 (供查询)
    pub fn last_status(&self, name: &str) -> Option<ServiceStatus> {
        self.last.get(name).copied()
    }

    /// 记录一个 ServiceS 已创建 (由 job 队列回报)
    fn handle_service_started(&mut self, name: String, conf: Unit) -> Vec<SmOutput> {
        self.pending.remove(&name);
        self.active.insert(name, conf);
        Vec::new()
    }

    /// 记录一个 ServiceS 已终止 (由 ServiceS 上报)
    fn handle_service_terminated(&mut self, name: String, status: ServiceStatus) -> Vec<SmOutput> {
        self.active.remove(&name);
        self.pending.remove(&name);
        self.last.insert(name.clone(), status);
        // 正常停止 / 成功退出时, 清空该 unit 的启动限流计数.
        if !status.failed {
            self.start_attempts.remove(&name);
        }
        Vec::new()
    }

    /// 判断一个 unit 是否已达启动限流
    fn start_limit_hit(&self, name: &str, conf: &Unit) -> bool {
        let interval = conf.start_limit_interval_sec;
        let burst = conf.start_limit_burst;
        if interval == 0 || burst == 0 {
            return false;
        }
        let cutoff = Instant::now() - Duration::from_secs(interval as u64);
        self.start_attempts
            .get(name)
            .is_some_and(|times| times.iter().filter(|t| **t >= cutoff).count() >= burst as usize)
    }

    /// 记录一次启动尝试 (用于启动限流)
    fn record_start_attempt(&mut self, name: &str, conf: &Unit) {
        let interval = conf.start_limit_interval_sec;
        if interval == 0 {
            return;
        }
        let cutoff = Instant::now() - Duration::from_secs(interval as u64);
        let times = self.start_attempts.entry(name.to_string()).or_default();
        while times.front().is_some_and(|t| *t < cutoff) {
            times.pop_front();
        }
        times.push_back(Instant::now());
    }

    /// 处理一条单个 unit 操作
    fn handle_unit(&mut self, cmd: ServiceCmdUnit) -> Vec<SmOutput> {
        if self.stopping {
            return vec![res_err("service-d exiting")];
        }
        let ServiceCmdUnit { name, conf, op } = cmd;
        match op {
            SdReqUnitOp::Start => self.handle_start_or_restart(name, conf, ServiceOp::Start),
            SdReqUnitOp::Stop => self.handle_stop(name, conf, ServiceOp::Stop),
            SdReqUnitOp::Kill => self.handle_stop(name, conf, ServiceOp::Kill),
            SdReqUnitOp::Restart => self.handle_start_or_restart(name, conf, ServiceOp::Restart),
            SdReqUnitOp::TryRestart => self.handle_try_restart(name),
            SdReqUnitOp::Reload => self.handle_reload(name, conf),
            SdReqUnitOp::ReloadOrRestart => {
                self.handle_start_or_restart(name, conf, ServiceOp::ReloadOrRestart)
            }
            SdReqUnitOp::TryReloadOrRestart => self.handle_try_reload_or_restart(name),
            SdReqUnitOp::ResetFailed => self.handle_reset_failed(name, conf),
            _ => vec![res_err("unsupported operation")],
        }
    }

    /// 处理批量启动 (default.target)
    fn handle_start_many(&mut self, services: Vec<(String, Unit)>) -> Vec<SmOutput> {
        if self.stopping {
            return vec![SmOutput::ReplyResult(Err(SysdError::Failed(
                "service-d exiting".to_string(),
            )))];
        }
        let mut actions = Vec::new();
        for (name, conf) in services {
            if self.active.contains_key(&name) {
                actions.push(SmOutput::SendServiceS {
                    name,
                    op: ServiceOp::Start,
                });
            } else if !self.pending.contains(&name) {
                self.pending.insert(name.clone());
                actions.push(SmOutput::EnqueueStart { name, conf });
            }
        }
        actions.push(SmOutput::ReplyResult(Ok(())));
        actions
    }

    /// 处理退出 (请求停止所有 service, 等待后台 task 回报)
    fn handle_exit(&mut self) -> Vec<SmOutput> {
        if self.stopping {
            return vec![SmOutput::ReplyResult(Err(SysdError::Failed(
                "service-d exiting".to_string(),
            )))];
        }
        self.stopping = true;
        vec![SmOutput::StopAll]
    }

    /// 处理后台 task 已完成停止所有 service (退出序列最后一步)
    fn handle_stop_all_done(&self, result: Result<(), SysdError>) -> Vec<SmOutput> {
        vec![SmOutput::ReplyResult(result), SmOutput::Exit]
    }

    /// start / restart / reload-or-restart 合并处理 (active 转发, 否则入队 start)
    fn handle_start_or_restart(
        &mut self,
        name: String,
        conf: Option<Unit>,
        active_op: ServiceOp,
    ) -> Vec<SmOutput> {
        if self.active.contains_key(&name) {
            return vec![SmOutput::SendServiceS {
                name,
                op: active_op,
            }];
        }
        if self.pending.contains(&name) {
            return vec![SmOutput::Reply(SdRes::Ok(None))];
        }
        match conf {
            Some(c) => {
                if self.start_limit_hit(&name, &c) {
                    self.last.insert(
                        name.clone(),
                        ServiceStatus {
                            failed: true,
                            restart_count: 0,
                            exit_status: None,
                        },
                    );
                    return vec![res_err(format!("Unit {} start-limit-hit.", name))];
                }
                self.record_start_attempt(&name, &c);
                self.pending.insert(name.clone());
                vec![SmOutput::EnqueueStart { name, conf: c }]
            }
            None => vec![res_err(not_found(&name))],
        }
    }

    /// stop / kill 合并处理
    fn handle_stop(&self, name: String, conf: Option<Unit>, op: ServiceOp) -> Vec<SmOutput> {
        if self.active.contains_key(&name) {
            return vec![SmOutput::SendServiceS { name, op }];
        }
        if self.last.contains_key(&name) || conf.is_some() || self.pending.contains(&name) {
            return vec![SmOutput::Reply(SdRes::Ok(None))];
        }
        vec![res_err(not_found(&name))]
    }

    /// try-restart: 仅 active 时转发, 否则 no-op
    fn handle_try_restart(&self, name: String) -> Vec<SmOutput> {
        if self.active.contains_key(&name) {
            vec![SmOutput::SendServiceS {
                name,
                op: ServiceOp::TryRestart,
            }]
        } else {
            vec![SmOutput::Reply(SdRes::Ok(None))]
        }
    }

    /// reload: 仅 active 时转发.
    /// 未运行直接报错 (与 systemd 一致).
    fn handle_reload(&self, name: String, conf: Option<Unit>) -> Vec<SmOutput> {
        if self.active.contains_key(&name) {
            return vec![SmOutput::SendServiceS {
                name,
                op: ServiceOp::Reload,
            }];
        }
        if conf.is_none() {
            return vec![res_err(not_found(&name))];
        }
        vec![res_err(format!("Unit {} is not running.", name))]
    }

    /// try-reload-or-restart: 仅 active 时转发, 否则 no-op
    fn handle_try_reload_or_restart(&self, name: String) -> Vec<SmOutput> {
        if self.active.contains_key(&name) {
            vec![SmOutput::SendServiceS {
                name,
                op: ServiceOp::TryReloadOrRestart,
            }]
        } else {
            vec![SmOutput::Reply(SdRes::Ok(None))]
        }
    }

    /// 清除 failed 状态 ("" 表示清除全部)
    fn handle_reset_failed(&mut self, name: String, conf: Option<Unit>) -> Vec<SmOutput> {
        if !name.is_empty()
            && conf.is_none()
            && !self.active.contains_key(&name)
            && !self.pending.contains(&name)
            && !self.last.contains_key(&name)
        {
            return vec![res_err(not_found(&name))];
        }

        if name.is_empty() {
            for s in self.last.values_mut() {
                s.failed = false;
            }
            self.start_attempts.clear();
        } else if let Some(s) = self.last.get_mut(&name) {
            s.failed = false;
            self.start_attempts.remove(&name);
        }

        if self.active.is_empty() {
            vec![SmOutput::Reply(SdRes::Ok(None))]
        } else {
            vec![SmOutput::SendServiceSAll {
                op: ServiceOp::ResetFailed,
            }]
        }
    }
}

/// 构造错误响应动作
fn res_err(msg: impl Into<String>) -> SmOutput {
    SmOutput::Reply(SdRes::Err(msg.into()))
}

/// unit 未找到的错误消息
fn not_found(name: &str) -> String {
    format!("Unit {} not found.", name)
}
