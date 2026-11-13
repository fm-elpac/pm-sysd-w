//! UnitD 状态机 (仅计算/决策, 不执行 IO).
//!
//! UnitDSM 持有 [`UnitS`] 与当前 target 作为状态. UnitD 把 [`UnitDM`] 消息喂给
//! 状态机, 再执行状态机返回的 [`SmActionR`] / [`SmActionD`] 列表. 状态机不接触
//! `oneshot::Sender`, 回执由 UnitD 在执行动作时注入/发送.

use crate::{
    conf::{Conf, Unit, UnitType},
    jsonl::{
        SdReq, SdReqUnit, SdReqUnitOp, SdReqUnitOpRO, SdReqUnitRO, SdReqUnitd, SdReqUnitdRO, SdRes,
    },
    sysd::{SysdError, TARGET_DEFAULT, TARGET_EXIT},
};

use crate::sysd::service_d::ServiceCmdUnit;

use super::{
    query::{QueryKind, QueryReq},
    unit_s::UnitS,
    unit_type_by_name,
};

/// 状态机输出的动作 (由 UnitD 执行)
///
/// 对应 `oneshot::Sender<Result<(), SysdError>>` 回执.
pub enum SmActionD {
    /// 写一条日志 (调用 logd)
    #[allow(dead_code)]
    Log {
        level: &'static str,
        unit: Option<String>,
        msg: String,
    },
    /// 立即返回 StartDefaultTarget / Exit 响应
    ///
    /// 当前实现把回执透传给 ServiceD (无阻塞转发), 故该变体暂时未使用
    #[allow(dead_code)]
    Reply(Result<(), SysdError>),
    /// 批量启动 (转发给 ServiceD)
    SendStartMany(Vec<(String, Unit)>),
    // 转发命令给 TimerD (预留)
    //SendTimer,
    /// 向 ServiceD 发送退出消息 (ServiceD 停止所有 service 并退出)
    ExitService,
    /// 退出 UnitD 主循环
    Exit,
}

/// 状态机输出的动作 (由 UnitD 执行)
///
/// 对应 `oneshot::Sender<SdRes>` 回执.
pub enum SmActionR {
    /// 写一条日志 (调用 logd)
    Log {
        level: &'static str,
        unit: Option<String>,
        msg: String,
    },
    /// 立即返回 SdReq 响应
    Reply(SdRes),
    /// 给自己发送进一步操作
    SendSelf(SdReq),
    /// 转发单个 unit 操作给 ServiceD
    SendService(ServiceCmdUnit),
    /// 转发命令给 TimerD (预留)
    //SendTimer,
    /// 查询 (组合 UnitD 与 ServiceD 数据), 完成后回复 SdReq 调用方
    QueryService(QueryReq),
    /// 写数据库 (mask / enable), 完成后回复 SdReq 调用方 (成功 Ok / 失败 Err)
    WriteDb(WriteUnit),
    /// daemon-reload
    Reload,
}

/// 向数据库写入 enable / mask 的单个更新
#[derive(Debug, Clone)]
pub struct WriteUnit {
    pub name: String,
    pub enabled: Option<bool>,
    pub masked: Option<bool>,
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
    Mask,
    Unmask,
}

/// UnitD 状态机
pub struct UnitDSM {
    /// 状态: unit 配置与 enable/mask 持久状态
    units: UnitS,
    /// 当前 target (运行中的 target), 默认 `default.target`
    current_target: String,
}

impl UnitDSM {
    /// 用 初始 UnitS 创建
    pub fn new(units: UnitS) -> Self {
        Self {
            units,
            current_target: TARGET_DEFAULT.to_string(),
        }
    }

    /// 处理启动默认 target 请求
    pub fn handle_start_default_target(&self) -> Vec<SmActionD> {
        let services: Vec<(String, Unit)> = self
            .units
            .target_units(TARGET_DEFAULT)
            .into_iter()
            .map(|name| {
                let conf = self.units.get(&name).cloned().expect("target unit exists");
                (name, conf)
            })
            .collect();
        vec![SmActionD::SendStartMany(services)]
    }

    /// 处理退出请求
    pub fn handle_exit(&mut self) -> Vec<SmActionD> {
        // 退出过程中先把当前 target 改成 `exit.target`
        self.current_target = TARGET_EXIT.to_string();
        vec![SmActionD::ExitService, SmActionD::Exit]
    }

    /// 处理 daemon-reload 加载完成
    pub fn handle_reload_ok(&mut self, conf: Conf) -> Vec<SmActionR> {
        // 用新配置重建 UnitS (daemon-reload 时整体替换)
        self.units = UnitS::new(conf.units, self.units.get_me());

        let mut a = Vec::new();
        for w in &conf.warnings {
            a.push(SmActionR::Log {
                level: "warn",
                unit: None,
                msg: w.clone(),
            });
        }
        a.push(SmActionR::Log {
            level: "info",
            unit: None,
            msg: "daemon-reload".to_string(),
        });
        a.push(SmActionR::Reply(SdRes::Ok));
        a
    }

    /// 处理一条 ctl 请求 (SdReq)
    pub fn handle_sd_req(&mut self, req: SdReq) -> Vec<SmActionR> {
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

    fn handle_unitd_ro(&mut self, req: SdReqUnitdRO) -> Vec<SmActionR> {
        match req {
            SdReqUnitdRO::GetDefault => {
                // 目前不支持 自定义 target, 所以固定写死
                vec![SmActionR::Reply(SdRes::DefaultTarget(
                    TARGET_DEFAULT.to_string(),
                ))]
            }
            SdReqUnitdRO::ListUnits => vec![SmActionR::QueryService(QueryReq {
                kind: QueryKind::ListUnits {
                    target: self.current_target.clone(),
                    infos: self.units.all_infos(),
                },
            })],
            SdReqUnitdRO::ListTimers => vec![SmActionR::Reply(SdRes::Timers(vec![]))],
            SdReqUnitdRO::ShowEnvironment => vec![res_err("TODO")],
        }
    }

    fn handle_unitd(&mut self, req: SdReqUnitd) -> Vec<SmActionR> {
        match req {
            SdReqUnitd::SetDefault { target } => {
                if target != TARGET_DEFAULT {
                    vec![res_err(format!("unknown target `{}`", target))]
                } else {
                    vec![SmActionR::Reply(SdRes::Ok)]
                }
            }
            SdReqUnitd::DaemonReload => vec![SmActionR::Reload],
            SdReqUnitd::SetEnvironment | SdReqUnitd::UnsetEnvironment => {
                vec![res_err("TODO")]
            }
        }
    }

    fn handle_unit_ro(&mut self, req: SdReqUnitRO) -> Vec<SmActionR> {
        let name = req.name;
        match req.op {
            SdReqUnitOpRO::UnitState => {
                let info = self.units.info_of(&name);
                vec![SmActionR::QueryService(QueryReq {
                    kind: QueryKind::UnitState { name, info },
                })]
            }
            SdReqUnitOpRO::Status => {
                let info = self.units.info_of(&name);
                vec![SmActionR::QueryService(QueryReq {
                    kind: QueryKind::Status { name, info },
                })]
            }
            SdReqUnitOpRO::Config => match self.units.get(&name) {
                Some(c) => vec![SmActionR::Reply(SdRes::Config(c.clone()))],
                None => vec![res_err(not_found_msg(&name))],
            },
            SdReqUnitOpRO::ListDependencies => {
                if self.units.contains(&name) {
                    vec![SmActionR::Reply(SdRes::Dependencies(vec![]))]
                } else {
                    vec![res_err(not_found_msg(&name))]
                }
            }
        }
    }

    fn handle_unit(&mut self, req: SdReqUnit) -> Vec<SmActionR> {
        let name = req.name;
        match req.op {
            SdReqUnitOp::Enable { now } => self.handle_enable_op(name, EnableOp::Enable { now }),
            SdReqUnitOp::Disable { now } => self.handle_enable_op(name, EnableOp::Disable { now }),
            SdReqUnitOp::Reenable => self.handle_enable_op(name, EnableOp::Reenable),
            SdReqUnitOp::Mask => self.handle_mask_op(name, MaskOp::Mask),
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
    fn handle_enable_op(&mut self, name: String, op: EnableOp) -> Vec<SmActionR> {
        // enable / reenable 写入 true, disable 写入 false
        let enabled = !matches!(op, EnableOp::Disable { .. });

        if enabled {
            // enable / reenable 需要校验 target 与 masked, 并读取 unit 配置
            let Some(unit) = self.units.get(&name).cloned() else {
                return vec![res_err(not_found_msg(&name))];
            };
            let masked = self.units.masked(&name);
            if let Err(e) = validate_enable(&unit, masked) {
                return vec![res_err(e)];
            }
        } else if !self.units.contains(&name) {
            return vec![res_err(not_found_msg(&name))];
        }

        self.units.set_enabled(&name, enabled);

        let msg = match op {
            EnableOp::Enable { .. } => "enabled",
            EnableOp::Disable { .. } => "disabled",
            EnableOp::Reenable => "reenabled",
        };

        let mut a = vec![
            SmActionR::WriteDb(WriteUnit {
                name: name.clone(),
                enabled: Some(enabled),
                masked: None,
            }),
            SmActionR::Log {
                level: "info",
                unit: Some(name.clone()),
                msg: msg.to_string(),
            },
        ];

        match op {
            EnableOp::Enable { now: true } => {
                a.push(SmActionR::SendSelf(SdReq::Unit(SdReqUnit {
                    name,
                    op: SdReqUnitOp::Start,
                })));
            }
            EnableOp::Disable { now: true } => {
                a.push(SmActionR::SendSelf(SdReq::Unit(SdReqUnit {
                    name,
                    op: SdReqUnitOp::Stop,
                })));
            }
            _ => {
                a.push(SmActionR::Reply(SdRes::Ok));
            }
        }
        a
    }

    /// mask / unmask 合并处理
    fn handle_mask_op(&mut self, name: String, op: MaskOp) -> Vec<SmActionR> {
        if !self.units.contains(&name) {
            return vec![res_err(not_found_msg(&name))];
        }
        let masked = matches!(op, MaskOp::Mask);
        self.units.set_masked(&name, masked);

        let mut a = vec![SmActionR::WriteDb(WriteUnit {
            name: name.clone(),
            masked: Some(masked),
            enabled: None,
        })];
        match op {
            // 先写数据库, 再停止 (若正在运行)
            MaskOp::Mask => a.push(SmActionR::SendSelf(SdReq::Unit(SdReqUnit {
                name,
                op: SdReqUnitOp::Stop,
            }))),
            MaskOp::Unmask => a.push(SmActionR::Reply(SdRes::Ok)),
        }
        a
    }

    /// 统一处理转发给 ServiceD 的单个 unit 操作
    fn handle_service_op(&self, name: String, op: SdReqUnitOp) -> Vec<SmActionR> {
        // ResetFailed 且 name 为空 = 清除全部, 无需 unit 类型判断.
        let reset_all = match &op {
            SdReqUnitOp::ResetFailed => name.is_empty(),
            _ => false,
        };
        if reset_all {
            return vec![SmActionR::SendService(ServiceCmdUnit {
                name,
                conf: None,
                op,
            })];
        }

        // 仅 Start 需要先检查 masked (与之前行为一致).
        let start = match &op {
            SdReqUnitOp::Start => true,
            _ => false,
        };

        if start && self.units.masked(&name) {
            return vec![res_err(format!("Unit {} is masked.", name))];
        }
        let conf = self.units.get(&name).cloned();
        let utype = conf
            .as_ref()
            .map(|c| Some(c.unit_type))
            .unwrap_or_else(|| unit_type_by_name(&name));
        match utype {
            Some(UnitType::Service) => {
                vec![SmActionR::SendService(ServiceCmdUnit { name, conf, op })]
            }
            // 暂不支持
            //UnitType::Timer => vec![SmActionR::SendTimer],
            //UnitType::Target => TODO,
            _ => vec![res_err(format!("cannot operate unit `{}`", name))],
        }
    }
}

/// 构造错误响应动作
fn res_err(msg: impl Into<String>) -> SmActionR {
    SmActionR::Reply(SdRes::Err(msg.into()))
}

/// unit 未找到的错误消息
fn not_found_msg(name: &str) -> String {
    format!("Unit {} not found.", name)
}

/// 校验一个 unit 是否可 enable
fn validate_enable(unit: &Unit, masked: bool) -> Result<(), String> {
    if unit.install.wanted_by.as_deref() != Some(TARGET_DEFAULT) {
        return Err(format!(
            "unit `{}` cannot be enabled (unknown target `{:?}`)",
            unit.name, unit.install.wanted_by,
        ));
    }
    if masked {
        return Err(format!("Unit {} is masked.", unit.name));
    }
    Ok(())
}
