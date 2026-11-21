//! UnitD 状态机 (纯计算).
//!
//! [`Sm`] 持有 [`UnitS`] 与当前 target 作为状态, 并用 `busy` / `exiting` 管理
//! 自身串行操作与退出状态. UnitD 把消息喂给状态机, 再执行状态机返回的
//! [`SmOutput`] 动作列表.
//!
//! 自身串行操作 busy 时输出 [`SmOutput::Wait`], 完成时输出 [`SmOutput::Idle`].
//! 等候队列由 actor 持有.
//!
//! 状态机不接触 `oneshot::Sender`, 回执由 UnitD actor 在执行动作时注入/发送.

use serde::Serialize;

use log::trace;

use crate::{
    conf::{Unit, UnitType},
    jsonl::{
        SdReq, SdReqUnit, SdReqUnitOp, SdReqUnitOpRO, SdReqUnitRO, SdReqUnitd, SdReqUnitdRO, SdRes,
        SdResPage,
    },
    sysd::{SysdError, TARGET_DEFAULT, TARGET_EXIT, reload::ReloadR, service_d::ServiceCmdUnit},
    sysu::{DaemonSta, EnableSta, LoadSta},
};

use super::{
    query::{QueryKind, QueryReq},
    unit_s::{UnitS, unit_type_by_name},
};

mod io;

pub use io::{PendingWrite, SmInput, SmOutput, WriteUnit};

/// UnitD 状态机.
///
/// 状态机的 `pub fn` 应该尽量少 (尽量避免直接读取状态机内部状态).
#[derive(Debug, Clone, Serialize)]
pub struct Sm {
    /// 状态: unit 配置与 enable/mask 持久状态
    units: UnitS,
    /// 当前 target (运行中的 target), 默认 `default.target`
    current_target: String,
    /// 配置文件路径 (用于 daemon-reload)
    conf_path: String,
    /// 是否正在处理一个自身串行操作 (同一时间只能有一个)
    busy: bool,
    /// 是否已进入退出状态 (不再接受新的自身串行操作)
    exiting: bool,
}

/// enable / disable / reenable 合并操作的参数
#[derive(Clone, Copy)]
enum EnableOp {
    Enable { now: bool },
    Disable { now: bool },
    Reenable,
}

/// mask / unmask 合并操作的参数
#[derive(Clone, Copy)]
enum MaskOp {
    Mask { now: bool },
    Unmask,
}

impl Sm {
    /// 用初始 UnitS 与配置文件路径创建
    pub fn new(units: UnitS, conf_path: String) -> Self {
        Self {
            units,
            current_target: TARGET_DEFAULT.to_string(),
            conf_path,
            busy: false,
            exiting: false,
        }
    }

    /// 状态机是否正在处理一个自身串行操作.
    ///
    /// 例外: 此处允许直接读取状态机内部状态.
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// 状态机主要处理逻辑 (入口)
    pub fn t(&mut self, input: SmInput) -> Vec<SmOutput> {
        trace!("in {:?}", input);
        let out = self.it(input);
        trace!("out {:?}", out);
        out
    }

    /// 状态机主要处理逻辑 (内部实现, 由 [`Sm::t`] 包装 trace)
    fn it(&mut self, input: SmInput) -> Vec<SmOutput> {
        match input {
            SmInput::SdReq(req) => {
                if is_self_serial(&req) {
                    if self.exiting {
                        vec![res_err("unitd exiting")]
                    } else if self.busy {
                        vec![SmOutput::Wait]
                    } else {
                        self.busy = true;
                        let mut a = self.handle_sd_req(req);
                        if !has_async_pending(&a) {
                            self.busy = false;
                            a.push(SmOutput::Idle);
                        }
                        a
                    }
                } else {
                    self.handle_sd_req(req)
                }
            }
            SmInput::StartDefaultTarget => self.handle_start_default_target(),
            SmInput::Exit => self.handle_exit(),
            SmInput::ReloadOk(r) => {
                let mut a = self.handle_reload_ok(r);
                self.busy = false;
                a.push(SmOutput::Idle);
                a
            }
            SmInput::ReloadErr(e) => {
                let mut a = self.handle_reload_err(e);
                self.busy = false;
                a.push(SmOutput::Idle);
                a
            }
            SmInput::WriteDbDone(result, pending) => {
                let mut a = self.handle_write_db_done(result, pending);
                self.busy = false;
                a.push(SmOutput::Idle);
                a
            }
            SmInput::ExitServiceDone(result) => self.handle_exit_service_done(result),
        }
    }

    /// 处理启动默认 target 请求
    fn handle_start_default_target(&self) -> Vec<SmOutput> {
        let services: Vec<(String, Unit)> = self
            .units
            .target_units(TARGET_DEFAULT)
            .into_iter()
            .map(|name| {
                let conf = self.units.get(&name).cloned().expect("target unit exists");
                (name, conf)
            })
            .collect();
        vec![SmOutput::SendStartMany(services)]
    }

    /// 处理退出请求
    fn handle_exit(&mut self) -> Vec<SmOutput> {
        // 退出过程中先把当前 target 改成 `exit.target`
        self.current_target = TARGET_EXIT.to_string();

        // 进入退出状态: 不再接受新的自身串行操作
        self.exiting = true;
        vec![SmOutput::ExitService]
    }

    /// 处理 ServiceD 已停止所有 service (退出序列最后一步)
    fn handle_exit_service_done(&self, result: Result<(), SysdError>) -> Vec<SmOutput> {
        vec![SmOutput::ReplyResult(result), SmOutput::Exit]
    }

    /// 处理 daemon-reload 加载完成
    fn handle_reload_ok(&mut self, r: ReloadR) -> Vec<SmOutput> {
        let ReloadR { conf, me } = r;
        // 用新配置与从数据库重新读取的 mask/enable 状态重建 UnitS
        self.units = UnitS::new(conf.units, me);

        let mut a = Vec::new();
        for w in &conf.warnings {
            a.push(SmOutput::Log {
                level: "warn",
                unit: None,
                msg: w.clone(),
            });
        }
        a.push(SmOutput::Log {
            level: "info",
            unit: None,
            msg: "daemon-reload".to_string(),
        });
        a.push(SmOutput::Reply(SdRes::Ok(None)));
        a
    }

    /// 处理 daemon-reload 加载失败
    fn handle_reload_err(&mut self, e: String) -> Vec<SmOutput> {
        vec![SmOutput::Reply(SdRes::Err(e))]
    }

    /// 处理数据库写入完成 (enable / mask 持久化结果)
    fn handle_write_db_done(
        &self,
        result: Result<(), SysdError>,
        pending: PendingWrite,
    ) -> Vec<SmOutput> {
        if let Err(e) = result {
            return vec![SmOutput::Reply(SdRes::Err(e.to_string()))];
        }

        match pending {
            PendingWrite::Enable { name, now } => {
                let mut a = vec![log_enabled(&name, "enabled")];
                if now {
                    a.push(SmOutput::SendSelf(SdReq::Unit(SdReqUnit {
                        name,
                        op: SdReqUnitOp::Start,
                    })));
                } else {
                    a.push(SmOutput::Reply(SdRes::Ok(None)));
                }
                a
            }
            PendingWrite::Disable { name, now } => {
                let mut a = vec![log_enabled(&name, "disabled")];
                if now {
                    a.push(SmOutput::SendSelf(SdReq::Unit(SdReqUnit {
                        name,
                        op: SdReqUnitOp::Stop,
                    })));
                } else {
                    a.push(SmOutput::Reply(SdRes::Ok(None)));
                }
                a
            }
            PendingWrite::Reenable { name } => {
                vec![
                    log_enabled(&name, "reenabled"),
                    SmOutput::Reply(SdRes::Ok(None)),
                ]
            }
            PendingWrite::Mask {
                name,
                stop,
                warning,
            } => {
                if stop {
                    vec![SmOutput::SendSelf(SdReq::Unit(SdReqUnit {
                        name,
                        op: SdReqUnitOp::Stop,
                    }))]
                } else {
                    vec![SmOutput::Reply(SdRes::Ok(warning))]
                }
            }
            PendingWrite::Unmask => vec![SmOutput::Reply(SdRes::Ok(None))],
        }
    }

    /// 处理一条 ctl 请求 (SdReq)
    fn handle_sd_req(&mut self, req: SdReq) -> Vec<SmOutput> {
        match req {
            SdReq::UnitdRO(r) => self.handle_unitd_ro(r),
            SdReq::UnitD(r) => self.handle_unitd(r),
            SdReq::UnitRO(r) => self.handle_unit_ro(r),
            SdReq::Unit(r) => self.handle_unit(r),
            SdReq::Whoami(w) => match w.pid {
                Some(pid) => vec![SmOutput::QueryService(QueryReq {
                    kind: QueryKind::Whoami { pid },
                })],
                // 无 pid 的 whoami 由 SysD 直接回复, 不应到达 UnitD.
                None => vec![res_err("whoami without pid")],
            },
            // 不支持其余请求
            _ => {
                vec![res_err("unsupported request")]
            }
        }
    }

    fn handle_unitd_ro(&mut self, req: SdReqUnitdRO) -> Vec<SmOutput> {
        match req {
            SdReqUnitdRO::GetDefault => {
                // 目前不支持自定义 target, 所以固定写死
                vec![SmOutput::Reply(SdRes::DefaultTarget(
                    TARGET_DEFAULT.to_string(),
                ))]
            }
            SdReqUnitdRO::ListUnits(q) => vec![SmOutput::QueryService(QueryReq {
                kind: QueryKind::ListUnits {
                    target: self.current_target.clone(),
                    infos: self.units.all_infos(),
                    limit: q.limit,
                    cursor: q.cursor,
                    masked: self.units.masked_names(),
                },
            })],
            SdReqUnitdRO::ListTimers => vec![SmOutput::Reply(SdRes::Timers(SdResPage {
                total: 0,
                cursor: None,
                list: vec![],
            }))],
            SdReqUnitdRO::SystemState => {
                if self.exiting {
                    vec![SmOutput::Reply(SdRes::SystemState(DaemonSta::Stopping))]
                } else {
                    vec![SmOutput::QueryService(QueryReq {
                        kind: QueryKind::SystemState {
                            infos: self.units.all_infos(),
                            masked: self.units.masked_names(),
                        },
                    })]
                }
            }
            SdReqUnitdRO::ShowEnvironment => vec![res_err("TODO")],
        }
    }

    fn handle_unitd(&mut self, req: SdReqUnitd) -> Vec<SmOutput> {
        match req {
            SdReqUnitd::SetDefault { target } => {
                if target != TARGET_DEFAULT {
                    vec![res_err(format!("unknown target `{}`", target))]
                } else {
                    vec![SmOutput::Reply(SdRes::Ok(None))]
                }
            }
            SdReqUnitd::DaemonReload => {
                vec![SmOutput::Reload(self.conf_path.clone())]
            }
            SdReqUnitd::SetEnvironment | SdReqUnitd::UnsetEnvironment => {
                vec![res_err("TODO")]
            }
        }
    }

    fn handle_unit_ro(&mut self, req: SdReqUnitRO) -> Vec<SmOutput> {
        let name = req.name;
        match req.op {
            SdReqUnitOpRO::UnitState => {
                let info = self.units.info_of(&name);
                vec![SmOutput::QueryService(QueryReq {
                    kind: QueryKind::UnitState { name, info },
                })]
            }
            SdReqUnitOpRO::Status => {
                let info = self.units.info_of(&name);
                vec![SmOutput::QueryService(QueryReq {
                    kind: QueryKind::Status { name, info },
                })]
            }
            SdReqUnitOpRO::Config => {
                // 被 mask 的 unit 视为不存在, 不返回配置详情.
                if self.units.masked(&name) {
                    vec![res_err(not_found_msg(&name))]
                } else {
                    match self.units.get(&name) {
                        Some(c) => vec![SmOutput::Reply(SdRes::Config(c.clone()))],
                        None => vec![res_err(not_found_msg(&name))],
                    }
                }
            }
            SdReqUnitOpRO::ListDependencies => {
                if self.units.masked(&name) || !self.units.contains(&name) {
                    vec![res_err(not_found_msg(&name))]
                } else {
                    vec![SmOutput::Reply(SdRes::Dependencies(SdResPage {
                        total: 0,
                        cursor: None,
                        list: vec![],
                    }))]
                }
            }
        }
    }

    fn handle_unit(&mut self, req: SdReqUnit) -> Vec<SmOutput> {
        let name = req.name;
        match req.op {
            SdReqUnitOp::Mask { now } => self.handle_mask_op(name, MaskOp::Mask { now }),
            SdReqUnitOp::Unmask => self.handle_mask_op(name, MaskOp::Unmask),
            SdReqUnitOp::Enable { now } => self.handle_enable_op(name, EnableOp::Enable { now }),
            SdReqUnitOp::Disable { now } => self.handle_enable_op(name, EnableOp::Disable { now }),
            SdReqUnitOp::Reenable => self.handle_enable_op(name, EnableOp::Reenable),
            SdReqUnitOp::Preset => self.handle_preset(name),
            // 统一转发 ServiceD
            op @ (SdReqUnitOp::Start
            | SdReqUnitOp::Stop
            | SdReqUnitOp::Kill
            | SdReqUnitOp::Restart
            | SdReqUnitOp::TryRestart
            | SdReqUnitOp::Reload
            | SdReqUnitOp::ReloadOrRestart
            | SdReqUnitOp::TryReloadOrRestart
            | SdReqUnitOp::ResetFailed) => self.handle_service_op(name, op),
            // 暂不支持
            SdReqUnitOp::Clean => vec![res_err("TODO")],
        }
    }

    /// preset 操作 (初版等效 disable).
    ///
    /// 对于不支持 enable/disable 的 unit (无 `[Install]` 配置), 忽略并返回成功.
    fn handle_preset(&mut self, name: String) -> Vec<SmOutput> {
        if !self.units.contains(&name) {
            return vec![res_err(not_found_msg(&name))];
        }
        if self.units.enable_sta(&name) == EnableSta::Static {
            return vec![SmOutput::Reply(SdRes::Ok(None))];
        }
        self.handle_enable_op(name, EnableOp::Disable { now: false })
    }

    /// enable / disable / reenable 合并处理
    fn handle_enable_op(&mut self, name: String, op: EnableOp) -> Vec<SmOutput> {
        // enable / reenable 写入 true, disable 写入 false
        let enabled = !matches!(op, EnableOp::Disable { .. });

        if enabled {
            // enable / reenable 需要校验 target 与 masked, 并读取 unit 配置
            let Some(unit) = self.units.get(&name).cloned() else {
                return vec![res_err(not_found_msg(&name))];
            };
            if let Err(e) = validate_enable(&unit, self.units.enable_sta(&name)) {
                return vec![res_err(e)];
            }
        } else if !self.units.contains(&name) {
            return vec![res_err(not_found_msg(&name))];
        }

        self.units.set_enabled(&name, enabled);
        let masked = self.units.masked(&name);

        let pending = match op {
            EnableOp::Enable { now } => PendingWrite::Enable {
                name: name.clone(),
                now,
            },
            EnableOp::Disable { now } => PendingWrite::Disable {
                name: name.clone(),
                now,
            },
            EnableOp::Reenable => PendingWrite::Reenable { name: name.clone() },
        };

        vec![SmOutput::WriteDb(
            WriteUnit {
                name,
                enabled,
                masked,
            },
            pending,
        )]
    }

    /// mask / unmask 合并处理
    fn handle_mask_op(&mut self, name: String, op: MaskOp) -> Vec<SmOutput> {
        match op {
            MaskOp::Mask { now } => {
                let exists = self.units.contains(&name);
                let warning = if exists {
                    None
                } else {
                    // unit 不存在时仍然 mask, 但输出警告 (与 systemd 一致).
                    Some(format!("Unit {} does not exist, proceeding anyway.", name))
                };

                self.units.set_masked(&name, true);
                let enabled = self.units.enabled(&name);
                let pending = PendingWrite::Mask {
                    name: name.clone(),
                    stop: exists && now,
                    warning,
                };

                vec![SmOutput::WriteDb(
                    WriteUnit {
                        name,
                        enabled,
                        masked: true,
                    },
                    pending,
                )]
            }
            MaskOp::Unmask => {
                // 只有当前配置中存在, 或已被 mask 的 unit 才能 unmask.
                if !self.units.masked(&name) && !self.units.contains(&name) {
                    return vec![res_err(not_found_msg(&name))];
                }
                self.units.set_masked(&name, false);
                let enabled = self.units.enabled(&name);
                let pending = PendingWrite::Unmask;

                vec![SmOutput::WriteDb(
                    WriteUnit {
                        name,
                        enabled,
                        masked: false,
                    },
                    pending,
                )]
            }
        }
    }

    /// 统一处理转发给 ServiceD 的单个 unit 操作
    fn handle_service_op(&self, name: String, op: SdReqUnitOp) -> Vec<SmOutput> {
        // ResetFailed 且 name 为空 = 清除全部, 无需 unit 类型判断.
        let reset_all = match &op {
            SdReqUnitOp::ResetFailed => name.is_empty(),
            _ => false,
        };
        if reset_all {
            return vec![SmOutput::SendService(ServiceCmdUnit {
                name,
                conf: None,
                op,
            })];
        }

        // 所有可能启动 (或重载后重启) 的 unit 操作, 对被 mask 的 unit 都要阻止.
        let may_start = matches!(
            &op,
            SdReqUnitOp::Start
                | SdReqUnitOp::Restart
                | SdReqUnitOp::TryRestart
                | SdReqUnitOp::ReloadOrRestart
                | SdReqUnitOp::TryReloadOrRestart
        );

        if may_start && self.units.load_sta(&name) == LoadSta::Masked {
            return vec![res_err(format!("Unit {} is masked.", name))];
        }
        let conf = self.units.get(&name).cloned();
        let utype = conf
            .as_ref()
            .map(|c| Some(c.unit_type))
            .unwrap_or_else(|| unit_type_by_name(&name));
        match utype {
            Some(UnitType::Service) => {
                vec![SmOutput::SendService(ServiceCmdUnit { name, conf, op })]
            }
            // 暂不支持
            //UnitType::Timer => vec![SmOutput::SendTimer],
            //UnitType::Target => TODO,
            _ => vec![res_err(format!("cannot operate unit `{}`", name))],
        }
    }
}

/// 判断 SdReq 是否为 UnitD 自身需要串行处理的操作 (非转发)
fn is_self_serial(req: &SdReq) -> bool {
    match req {
        // UnitD 组件的修改操作都由 UnitD 自身处理
        SdReq::UnitD(_) => true,
        // 单个 unit 的修改操作中, 除转发 ServiceD 的操作外, 都由 UnitD 自身处理
        SdReq::Unit(SdReqUnit { op, .. }) => !is_forwarded_unit_op(op),
        _ => false,
    }
}

/// 判断单个 unit 修改操作是否应转发 ServiceD
fn is_forwarded_unit_op(op: &SdReqUnitOp) -> bool {
    matches!(
        op,
        SdReqUnitOp::Start
            | SdReqUnitOp::Stop
            | SdReqUnitOp::Kill
            | SdReqUnitOp::Restart
            | SdReqUnitOp::TryRestart
            | SdReqUnitOp::Reload
            | SdReqUnitOp::ReloadOrRestart
            | SdReqUnitOp::TryReloadOrRestart
            | SdReqUnitOp::ResetFailed
    )
}

/// 判断动作列表是否启动了异步后台任务 (自身串行操作尚未完成)
fn has_async_pending(a: &[SmOutput]) -> bool {
    a.iter()
        .any(|o| matches!(o, SmOutput::Reload(..) | SmOutput::WriteDb(..)))
}

/// 构造 enable / disable / reenable 的日志动作
fn log_enabled(name: &str, msg: &'static str) -> SmOutput {
    SmOutput::Log {
        level: "info",
        unit: Some(name.to_string()),
        msg: msg.to_string(),
    }
}

/// 校验一个 unit 是否可 enable
fn validate_enable(unit: &Unit, enable: EnableSta) -> Result<(), String> {
    // TODO: sysd 暂只支持一个 WantedBy, 多行语法只取第一个.
    if unit.install.wanted_by.first().map(|s| s.as_str()) != Some(TARGET_DEFAULT) {
        return Err(format!(
            "unit `{}` cannot be enabled (unknown target `{:?}`)",
            unit.name, unit.install.wanted_by,
        ));
    }
    if enable == EnableSta::Masked {
        return Err(format!("Unit {} is masked.", unit.name));
    }
    Ok(())
}

/// 构造错误响应动作
fn res_err(msg: impl Into<String>) -> SmOutput {
    SmOutput::Reply(SdRes::Err(msg.into()))
}

/// unit 未找到的错误消息
fn not_found_msg(name: &str) -> String {
    format!("Unit {} not found.", name)
}

#[cfg(test)]
mod test;
