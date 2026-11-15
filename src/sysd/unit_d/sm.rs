//! UnitD 状态机 (纯计算).
//!
//! [`Sm`] 持有 [`UnitS`] 与当前 target 作为状态. UnitD 把消息喂给状态机, 再执行
//! 状态机返回的 [`SmOutput`] 动作列表. 状态机不接触 `oneshot::Sender`, 回执由
//! UnitD actor 在执行动作时注入/发送.

use crate::{
    conf::{Conf, Unit, UnitType},
    jsonl::{
        SdReq, SdReqUnit, SdReqUnitOp, SdReqUnitOpRO, SdReqUnitRO, SdReqUnitd, SdReqUnitdRO, SdRes,
    },
    sysd::{EnableSta, LoadSta, SysdError, TARGET_DEFAULT, TARGET_EXIT},
};

use crate::sysd::service_d::ServiceCmdUnit;

use super::{
    query::{QueryKind, QueryReq},
    unit_s::{UnitS, UnitSME, unit_type_by_name},
};

/// UnitD 状态机
pub struct Sm {
    /// 状态: unit 配置与 enable/mask 持久状态
    units: UnitS,
    /// 当前 target (运行中的 target), 默认 `default.target`
    current_target: String,
    /// 是否正在 daemon-reload (reload 期间禁止再次 reload 与 mask/enable 修改)
    reloading: bool,
}

/// 状态机输入数据 (事件)
pub enum SmInput {
    /// 处理一条 ctl 请求
    SdReq(SdReq),
    /// daemon-reload 加载完成 (携带配置与从数据库重新读取的 mask/enable 状态)
    ReloadOk(Conf, UnitSME),
    /// daemon-reload 加载失败 (携带错误消息)
    ReloadErr(String),
    /// 启动默认 target 中已 enable 的 service
    StartDefaultTarget,
    /// 停止所有 unit, 然后退出 actor 主循环
    Exit,
    /// 数据库写入完成 (携带结果与对应的后续操作)
    WriteDbDone(Result<(), SysdError>, PendingWrite),
    /// ServiceD 已停止所有 service (退出序列内部事件)
    ExitServiceDone(Result<(), SysdError>),
}

/// 状态机输出的动作 (由 UnitD actor 执行)
pub enum SmOutput {
    /// 写一条日志 (调用 logd)
    Log {
        level: &'static str,
        unit: Option<String>,
        msg: String,
    },
    /// 立即返回 SdReq 响应
    Reply(SdRes),
    /// 立即返回 StartDefaultTarget / Exit 响应
    ReplyResult(Result<(), SysdError>),
    /// 给自己发送进一步操作
    SendSelf(SdReq),
    /// 转发单个 unit 操作给 ServiceD
    SendService(ServiceCmdUnit),
    /// 查询 (组合 UnitD 与 ServiceD 数据), 完成后回复 SdReq 调用方
    QueryService(QueryReq),
    /// 写数据库 (mask / enable), 携带完成后需要执行的后续操作; 由后台 task 回传
    WriteDb(WriteUnit, PendingWrite),
    /// daemon-reload
    Reload,
    /// 批量启动 (转发给 ServiceD)
    SendStartMany(Vec<(String, Unit)>),
    // 转发命令给 TimerD (预留)
    //SendTimer,
    /// 向 ServiceD 发送退出消息 (ServiceD 停止所有 service 并退出)
    ExitService,
    /// 退出 UnitD 主循环
    Exit,
}

/// 向数据库写入 enable / mask 的单个更新
#[derive(Debug, Clone)]
pub struct WriteUnit {
    pub name: String,
    pub enabled: Option<bool>,
    pub masked: Option<bool>,
}

/// 等待数据库写入完成后需要执行的后续操作
pub enum PendingWrite {
    /// enable (now 表示 --now)
    Enable { name: String, now: bool },
    /// disable (now 表示 --now)
    Disable { name: String, now: bool },
    /// reenable
    Reenable { name: String },
    /// mask (stop 表示 --now 且 unit 存在; warning 为 unit 不存在时的警告)
    Mask {
        name: String,
        stop: bool,
        warning: Option<String>,
    },
    /// unmask
    Unmask,
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
    /// 用初始 UnitS 创建
    pub fn new(units: UnitS) -> Self {
        Self {
            units,
            current_target: TARGET_DEFAULT.to_string(),
            reloading: false,
        }
    }

    /// 状态机主要处理逻辑 (入口)
    pub fn t(mut self, input: SmInput) -> (Self, Vec<SmOutput>) {
        let actions = match input {
            SmInput::SdReq(req) => self.handle_sd_req(req),
            SmInput::ReloadOk(conf, me) => self.handle_reload_ok(conf, me),
            SmInput::ReloadErr(e) => self.handle_reload_err(e),
            SmInput::StartDefaultTarget => self.handle_start_default_target(),
            SmInput::Exit => self.handle_exit(),
            SmInput::WriteDbDone(result, pending) => self.handle_write_db_done(result, pending),
            SmInput::ExitServiceDone(result) => self.handle_exit_service_done(result),
        };
        (self, actions)
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
        vec![SmOutput::ExitService]
    }

    /// 处理 ServiceD 已停止所有 service (退出序列最后一步)
    fn handle_exit_service_done(&self, result: Result<(), SysdError>) -> Vec<SmOutput> {
        vec![SmOutput::ReplyResult(result), SmOutput::Exit]
    }

    /// 处理 daemon-reload 加载完成
    fn handle_reload_ok(&mut self, conf: Conf, me: UnitSME) -> Vec<SmOutput> {
        // 用新配置与从数据库重新读取的 mask/enable 状态重建 UnitS
        self.units = UnitS::new(conf.units, me);
        self.reloading = false;

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
        self.reloading = false;
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
            SdReqUnitdRO::ListUnits => vec![SmOutput::QueryService(QueryReq {
                kind: QueryKind::ListUnits {
                    target: self.current_target.clone(),
                    infos: self.units.all_infos(),
                },
            })],
            SdReqUnitdRO::ListTimers => vec![SmOutput::Reply(SdRes::Timers(vec![]))],
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
                if self.reloading {
                    vec![res_err("daemon-reload already in progress")]
                } else {
                    self.reloading = true;
                    vec![SmOutput::Reload]
                }
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
                    vec![SmOutput::Reply(SdRes::Dependencies(vec![]))]
                }
            }
        }
    }

    fn handle_unit(&mut self, req: SdReqUnit) -> Vec<SmOutput> {
        let name = req.name;
        match req.op {
            SdReqUnitOp::Enable { now } => self.handle_enable_op(name, EnableOp::Enable { now }),
            SdReqUnitOp::Disable { now } => self.handle_enable_op(name, EnableOp::Disable { now }),
            SdReqUnitOp::Reenable => self.handle_enable_op(name, EnableOp::Reenable),
            SdReqUnitOp::Mask { now } => self.handle_mask_op(name, MaskOp::Mask { now }),
            SdReqUnitOp::Unmask => self.handle_mask_op(name, MaskOp::Unmask),
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
            SdReqUnitOp::Preset | SdReqUnitOp::Clean => vec![res_err("TODO")],
        }
    }

    /// enable / disable / reenable 合并处理
    fn handle_enable_op(&mut self, name: String, op: EnableOp) -> Vec<SmOutput> {
        if self.reloading {
            return vec![res_err("daemon-reload in progress")];
        }

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
                enabled: Some(enabled),
                masked: None,
            },
            pending,
        )]
    }

    /// mask / unmask 合并处理
    fn handle_mask_op(&mut self, name: String, op: MaskOp) -> Vec<SmOutput> {
        if self.reloading {
            return vec![res_err("daemon-reload in progress")];
        }

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
                let pending = PendingWrite::Mask {
                    name: name.clone(),
                    stop: exists && now,
                    warning,
                };

                vec![SmOutput::WriteDb(
                    WriteUnit {
                        name,
                        masked: Some(true),
                        enabled: None,
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
                let pending = PendingWrite::Unmask;

                vec![SmOutput::WriteDb(
                    WriteUnit {
                        name,
                        masked: Some(false),
                        enabled: None,
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

/// 构造 enable / disable / reenable 的日志动作
fn log_enabled(name: &str, msg: &'static str) -> SmOutput {
    SmOutput::Log {
        level: "info",
        unit: Some(name.to_string()),
        msg: msg.to_string(),
    }
}

/// 构造错误响应动作
fn res_err(msg: impl Into<String>) -> SmOutput {
    SmOutput::Reply(SdRes::Err(msg.into()))
}

/// unit 未找到的错误消息
fn not_found_msg(name: &str) -> String {
    format!("Unit {} not found.", name)
}

/// 校验一个 unit 是否可 enable
fn validate_enable(unit: &Unit, enable: EnableSta) -> Result<(), String> {
    if unit.install.wanted_by.as_deref() != Some(TARGET_DEFAULT) {
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
