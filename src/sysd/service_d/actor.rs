//! ServiceD 状态机 (仅计算/决策, 不执行 IO).
//!
//! ServiceDSM 持有活跃 service 的配置 (`name -> Unit`), 已入队待启动的 name, 以及
//! 最近终止的 service 状态. ServiceD 把 [`super::ServiceDM`] 消息喂给状态机, 再执行
//! 状态机返回的 [`SmActionR`] / [`SmActionD`] 列表. 状态机不持有 ServiceS 句柄,
//! 也不接触 logd / 消息发送通道; ServiceS 句柄由 ServiceD 主循环持有.

use std::collections::{HashMap, HashSet};

use tokio::sync::oneshot;

use crate::{
    conf::Unit,
    jsonl::{SdReqUnitOp, SdRes},
    sysd::{
        SysdError,
        service_s::{ServiceSM, ServiceStatus},
    },
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

impl ServiceOp {
    /// 构造携带回执的 [`ServiceSM`] 消息
    pub fn into_sm(self, reply: oneshot::Sender<Result<(), SysdError>>) -> ServiceSM {
        ServiceSM::Cmd(self, reply)
    }
}

/// 状态机输出的动作 (由 ServiceD 执行), 对应 `oneshot::Sender<SdRes>` 回执
pub enum SmActionR {
    /// 立即返回 SdReq 响应
    Reply(SdRes),
    /// 转发命令给一个 ServiceS (异步执行后回复)
    SendServiceS { name: String, op: ServiceOp },
    /// 转发命令给全部 ServiceS (异步执行后回复)
    SendServiceSAll { op: ServiceOp },
    /// 入队 start job (创建 ServiceS 并 start), 完成后回复
    EnqueueStart { name: String, conf: Unit },
}

/// 状态机输出的动作 (由 ServiceD 执行), 对应 `oneshot::Sender<Result<(), SysdError>>` 回执
pub enum SmActionD {
    /// 立即返回响应
    ///
    /// 当前实现把回执透传给 ServiceD (无阻塞转发), 故该变体暂时未使用.
    #[allow(dead_code)]
    Reply(Result<(), SysdError>),
    /// 转发命令给一个 ServiceS (不等待结果, 失败仅记录日志)
    SendServiceS { name: String, op: ServiceOp },
    /// 入队 start job (批量启动, 不单独回复)
    EnqueueStart { name: String, conf: Unit },
    /// 停止所有 service (转发 Stop 给全部 ServiceS, 等待完成)
    StopAll,
    /// 退出 ServiceD 主循环
    Exit,
}

/// ServiceD 状态机
pub struct ServiceDSM {
    /// 活跃 service 的配置 (name -> conf). 对应一个已创建的 ServiceS.
    active: HashMap<String, Unit>,
    /// 已入队 start job 但尚未创建 ServiceS 的 name
    pending: HashSet<String>,
    /// 最近终止 service 的状态
    last: HashMap<String, ServiceStatus>,
}

impl ServiceDSM {
    /// 创建空的状态机
    pub fn new() -> Self {
        Self {
            active: HashMap::new(),
            pending: HashSet::new(),
            last: HashMap::new(),
        }
    }

    /// 记录一个 ServiceS 已创建 (由 job 队列回报)
    pub fn started(&mut self, name: String, conf: Unit) {
        self.pending.remove(&name);
        self.active.insert(name, conf);
    }

    /// 记录一个 ServiceS 已终止 (由 ServiceS 上报)
    pub fn terminated(&mut self, name: &str, status: ServiceStatus) {
        self.active.remove(name);
        self.pending.remove(name);
        self.last.insert(name.to_string(), status);
    }

    /// 读取运行中 service 的旧配置
    pub fn running_unit(&self, name: &str) -> Option<Unit> {
        self.active.get(name).cloned()
    }

    /// 读取最近终止状态 (供查询)
    pub fn last_status(&self, name: &str) -> Option<ServiceStatus> {
        self.last.get(name).copied()
    }

    /// 处理一条单个 unit 操作
    pub fn handle_unit(&mut self, cmd: ServiceCmdUnit) -> Vec<SmActionR> {
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
    pub fn handle_start_many(&mut self, services: Vec<(String, Unit)>) -> Vec<SmActionD> {
        let mut actions = Vec::new();
        for (name, conf) in services {
            if self.active.contains_key(&name) {
                actions.push(SmActionD::SendServiceS {
                    name,
                    op: ServiceOp::Start,
                });
            } else if !self.pending.contains(&name) {
                self.pending.insert(name.clone());
                actions.push(SmActionD::EnqueueStart { name, conf });
            }
        }
        actions.push(SmActionD::Reply(Ok(())));
        actions
    }

    /// 处理退出 (停止所有 service 后退出主循环)
    pub fn handle_exit(&self) -> Vec<SmActionD> {
        vec![
            SmActionD::StopAll,
            SmActionD::Reply(Ok(())),
            SmActionD::Exit,
        ]
    }

    /// start / restart / reload-or-restart 合并处理 (active 转发, 否则入队 start)
    fn handle_start_or_restart(
        &mut self,
        name: String,
        conf: Option<Unit>,
        active_op: ServiceOp,
    ) -> Vec<SmActionR> {
        if self.active.contains_key(&name) {
            return vec![SmActionR::SendServiceS {
                name,
                op: active_op,
            }];
        }
        if self.pending.contains(&name) {
            return vec![SmActionR::Reply(SdRes::Ok)];
        }
        match conf {
            Some(c) => {
                self.pending.insert(name.clone());
                vec![SmActionR::EnqueueStart { name, conf: c }]
            }
            None => vec![res_err(not_found(&name))],
        }
    }

    /// stop / kill 合并处理
    fn handle_stop(&self, name: String, conf: Option<Unit>, op: ServiceOp) -> Vec<SmActionR> {
        if self.active.contains_key(&name) {
            return vec![SmActionR::SendServiceS { name, op }];
        }
        if self.last.contains_key(&name) || conf.is_some() || self.pending.contains(&name) {
            return vec![SmActionR::Reply(SdRes::Ok)];
        }
        vec![res_err(not_found(&name))]
    }

    /// try-restart: 仅 active 时转发, 否则 no-op
    fn handle_try_restart(&self, name: String) -> Vec<SmActionR> {
        if self.active.contains_key(&name) {
            vec![SmActionR::SendServiceS {
                name,
                op: ServiceOp::TryRestart,
            }]
        } else {
            vec![SmActionR::Reply(SdRes::Ok)]
        }
    }

    /// reload: 仅 active 时转发.
    /// 未运行直接报错 (与 systemd 一致).
    fn handle_reload(&self, name: String, conf: Option<Unit>) -> Vec<SmActionR> {
        if self.active.contains_key(&name) {
            return vec![SmActionR::SendServiceS {
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
    fn handle_try_reload_or_restart(&self, name: String) -> Vec<SmActionR> {
        if self.active.contains_key(&name) {
            vec![SmActionR::SendServiceS {
                name,
                op: ServiceOp::TryReloadOrRestart,
            }]
        } else {
            vec![SmActionR::Reply(SdRes::Ok)]
        }
    }

    /// 清除 failed 状态 ("" 表示清除全部)
    fn handle_reset_failed(&mut self, name: String, conf: Option<Unit>) -> Vec<SmActionR> {
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
        } else if let Some(s) = self.last.get_mut(&name) {
            s.failed = false;
        }

        if self.active.is_empty() {
            vec![SmActionR::Reply(SdRes::Ok)]
        } else {
            vec![SmActionR::SendServiceSAll {
                op: ServiceOp::ResetFailed,
            }]
        }
    }
}

/// 构造错误响应动作
fn res_err(msg: impl Into<String>) -> SmActionR {
    SmActionR::Reply(SdRes::Err(msg.into()))
}

/// unit 未找到的错误消息
fn not_found(name: &str) -> String {
    format!("Unit {} not found.", name)
}
