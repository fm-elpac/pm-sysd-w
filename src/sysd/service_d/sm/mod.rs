//! ServiceD 状态机 (纯计算).
//!
//! [`Sm`] 持有活跃 service 的配置 (`name -> Unit`), 以及最近终止的 service 状态.
//! ServiceD 把 [`super::ServiceDM`] 消息与内部事件 (ServiceS 退出请求) 喂给状态机,
//! 再执行状态机返回的 [`SmOutput`] 动作列表.
//!
//! 状态机不持有 ServiceS 句柄, 也不接触 logd / 消息发送通道.
//! ServiceS 句柄由 ServiceD actor 持有.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

use serde::{Serialize, Serializer, ser::SerializeStruct};

use log::trace;

use crate::{
    conf::Unit,
    jsonl::{SdReqUnitOp, SdRes},
    logd::{instant_iso, now_millis},
    sysd::{SysdError, service_s::ServiceStatus},
};

use super::ServiceCmdUnit;

mod io;

pub use io::{ServiceOp, SmInput, SmOutput};

/// ServiceD 状态机
#[derive(Debug, Clone)]
pub struct Sm {
    /// 活跃 service 的配置 (name -> conf). 对应一个已创建的 ServiceS.
    active: HashMap<String, Unit>,
    /// 最近终止 service 的状态
    last: HashMap<String, ServiceStatus>,
    /// 手动 start 的启动尝试时间戳 (StartLimitIntervalSec / StartLimitBurst)
    start_attempts: HashMap<String, VecDeque<Instant>>,
    /// 是否正在退出 (等待所有 service 停止)
    stopping: bool,
}

impl Serialize for Sm {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("Sm", 4)?;
        s.serialize_field("active", &self.active)?;
        s.serialize_field("last", &self.last)?;
        let start_attempts: HashMap<&str, Vec<String>> = self
            .start_attempts
            .iter()
            .map(|(name, times)| (name.as_str(), times.iter().map(instant_iso).collect()))
            .collect();
        s.serialize_field("start_attempts", &start_attempts)?;
        s.serialize_field("stopping", &self.stopping)?;
        s.end()
    }
}

impl Sm {
    /// 初始状态
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
            last: HashMap::new(),
            start_attempts: HashMap::new(),
            stopping: false,
        }
    }

    /// 状态机主要处理逻辑 (入口)
    pub fn t(self, input: SmInput) -> (Self, Vec<SmOutput>) {
        trace!("in {:?}", input);
        let (s, out) = self.it(input);
        trace!("out {:?}", out);
        (s, out)
    }

    /// 状态机主要处理逻辑 (内部实现, 由 [`Sm::t`] 包装 trace)
    fn it(mut self, input: SmInput) -> (Self, Vec<SmOutput>) {
        let actions = match input {
            SmInput::Unit(cmd) => self.handle_unit(cmd),
            SmInput::StartMany(services) => self.handle_start_many(services),
            SmInput::StartManyDone(result) => self.handle_start_many_done(result),
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

    /// 读取全部运行中 service 的旧配置 (按名称排序, 供 list-units 补齐)
    pub fn running_units(&self) -> Vec<(String, Unit)> {
        let mut v: Vec<(String, Unit)> = self
            .active
            .iter()
            .map(|(name, conf)| (name.clone(), conf.clone()))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    /// 读取最近终止状态 (供查询)
    pub fn last_status(&self, name: &str) -> Option<ServiceStatus> {
        self.last.get(name).copied()
    }

    /// 记录一个 ServiceS 已创建 (由 ServiceD actor 直接创建)
    fn handle_service_started(&mut self, name: String, conf: Unit) -> Vec<SmOutput> {
        self.active.insert(name, conf);
        Vec::new()
    }

    /// 记录一个 ServiceS 已终止 (由 ServiceS 请求退出)
    fn handle_service_terminated(&mut self, name: String, status: ServiceStatus) -> Vec<SmOutput> {
        self.active.remove(&name);
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
        let mut started = HashSet::new();
        for (name, conf) in services {
            if self.active.contains_key(&name) {
                actions.push(SmOutput::SendServiceS {
                    name,
                    op: ServiceOp::Start,
                });
            } else if started.insert(name.clone()) {
                actions.push(SmOutput::StartService { name, conf });
            }
        }
        actions
    }

    /// 处理批量启动全部完成
    fn handle_start_many_done(&self, result: Result<(), SysdError>) -> Vec<SmOutput> {
        vec![SmOutput::ReplyResult(result)]
    }

    /// 处理退出 (请求停止所有 service, 由 actor 等待句柄移除后完成)
    fn handle_exit(&mut self) -> Vec<SmOutput> {
        if self.stopping {
            return vec![SmOutput::ReplyResult(Err(SysdError::Failed(
                "service-d exiting".to_string(),
            )))];
        }
        self.stopping = true;
        vec![SmOutput::StopAll]
    }

    /// 处理所有 service 已停止 (退出序列最后一步)
    fn handle_stop_all_done(&self, result: Result<(), SysdError>) -> Vec<SmOutput> {
        vec![SmOutput::ReplyResult(result), SmOutput::Exit]
    }

    /// start / restart / reload-or-restart 合并处理 (active 转发, 否则直接创建)
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
        match conf {
            Some(c) => {
                if self.start_limit_hit(&name, &c) {
                    self.last.insert(
                        name.clone(),
                        ServiceStatus {
                            failed: true,
                            restart_count: 0,
                            exit_status: None,
                            since: Some(now_millis()),
                        },
                    );
                    return vec![res_err(format!("Unit {} start-limit-hit.", name))];
                }
                self.record_start_attempt(&name, &c);
                vec![SmOutput::StartService { name, conf: c }]
            }
            None => vec![res_err(not_found(&name))],
        }
    }

    /// stop / kill 合并处理
    fn handle_stop(&self, name: String, conf: Option<Unit>, op: ServiceOp) -> Vec<SmOutput> {
        if self.active.contains_key(&name) {
            return vec![SmOutput::SendServiceS { name, op }];
        }
        if self.last.contains_key(&name) || conf.is_some() {
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
            && !self.last.contains_key(&name)
        {
            return vec![res_err(not_found(&name))];
        }

        if name.is_empty() {
            for s in self.last.values_mut() {
                s.failed = false;
                s.since = Some(now_millis());
            }
            self.start_attempts.clear();
        } else if let Some(s) = self.last.get_mut(&name) {
            s.failed = false;
            s.since = Some(now_millis());
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

#[cfg(test)]
mod test;
