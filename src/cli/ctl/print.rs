//! ctl 命令 响应 的 格式化/打印.

use serde_json;

use crate::{
    conf::Unit,
    jsonl::{
        SdReq, SdReqUnitOp, SdReqUnitd, SdRes, SdResTimerInfo, SdResUnitS, SdResUnitState,
        SdResUnitStatus,
    },
    sysd::{UnitSta, UnitSubSta},
};

pub fn print_response(cmd_name: &str, cmd: &SdReq, resp: SdRes) -> u8 {
    match resp {
        SdRes::Ok => {
            if let Some(m) = action_ok_message(cmd) {
                println!("{}", m);
            }
            0
        }
        SdRes::Err(message) => {
            eprintln!("{}", message);
            1
        }
        SdRes::ListUnits(units) => {
            print_list_units(&units);
            0
        }
        SdRes::Status(status) => {
            print_status(&status);
            0
        }
        SdRes::Config(config) => {
            print_config(cmd_name, &config);
            0
        }
        SdRes::UnitState(state) => print_unit_state(cmd_name, state),
        SdRes::DefaultTarget(target) => {
            println!("{}", target);
            0
        }
        SdRes::Dependencies(deps) => {
            for d in &deps {
                println!("{}", d);
            }
            0
        }
        SdRes::Timers(timers) => {
            print_timers(&timers);
            0
        }
        SdRes::Logs { .. } => {
            // ctl 不会请求日志; 视为协议异常.
            eprintln!("unexpected response: logs");
            1
        }
    }
}

/// 操作成功时 的 提示消息 (大部分动作 静默成功, 与 systemctl 风格一致).
pub fn action_ok_message(cmd: &SdReq) -> Option<String> {
    match cmd {
        SdReq::Unit(u) => match &u.op {
            SdReqUnitOp::Enable { .. } => Some(format!("Enabled {}.", u.name)),
            SdReqUnitOp::Disable { .. } => Some(format!("Disabled {}.", u.name)),
            SdReqUnitOp::Reenable => Some(format!("Reenabled {}.", u.name)),
            SdReqUnitOp::Mask => Some(format!("Masked {}.", u.name)),
            SdReqUnitOp::Unmask => Some(format!("Unmasked {}.", u.name)),
            SdReqUnitOp::ResetFailed => {
                if u.name.is_empty() {
                    Some("Reset all failed states.".to_string())
                } else {
                    Some(format!("Reset failed state of {}.", u.name))
                }
            }
            _ => None,
        },
        SdReq::UnitD(SdReqUnitd::SetDefault { target }) => {
            Some(format!("Default target set to {}.", target))
        }
        SdReq::UnitD(SdReqUnitd::DaemonReload) => Some("Reloaded configuration.".to_string()),
        _ => None,
    }
}

/// 打印 `list-units` 结果.
pub fn print_list_units(units: &[SdResUnitState]) {
    println!("UNIT LOAD ACTIVE SUB DESCRIPTION");
    for u in units {
        println!(
            "{} loaded {} {} {}",
            u.name,
            active_str(u.active),
            sub_str(u.sub),
            u.description
        );
    }
}

/// 打印 `status` 结果.
pub fn print_status(s: &SdResUnitStatus) {
    println!("{} - {}", s.name, s.description);
    let load = if s.masked {
        "masked"
    } else if s.enabled {
        "enabled"
    } else {
        "disabled"
    };
    println!("   Loaded: {} ({})", "loaded", load);
    match &s.s {
        SdResUnitS::Service(ss) => {
            match ss.pid {
                Some(pid) => println!(
                    "   Active: {} ({}) (pid {})",
                    active_str(s.active),
                    ss.sub.as_str(),
                    pid
                ),
                None => println!("   Active: {} ({})", active_str(s.active), ss.sub.as_str()),
            }
            if ss.restart_count > 0 {
                println!("   Restarts: {}", ss.restart_count);
            }
            if let Some(code) = ss.exit_status {
                println!("   ExitStatus: {}", code);
            }
        }
        SdResUnitS::Timer(_) => {
            println!("   Active: {}", active_str(s.active));
        }
    }
}

/// 把 命令 (`Vec<String>`) 格式化为 单行 JSON.
pub fn cmd_json(cmd: &[String]) -> String {
    serde_json::to_string(cmd).unwrap_or_default()
}

/// 打印 unit 配置 查询结果.
pub fn print_config(cmd_name: &str, c: &Unit) {
    match cmd_name {
        "cat" => print_cat_json(c),
        _ => print_show(c),
    }
}

/// 打印 `show` 结果 (`Key=Value` 格式).
pub fn print_show(c: &Unit) {
    println!("Name={}", c.name);
    println!("Description={}", c.description);
    println!("Type={}", c.service.service_type.as_str());
    println!("ExecStart={}", cmd_json(&c.service.exec_start));
    if let Some(s) = &c.service.exec_stop {
        println!("ExecStop={}", cmd_json(s));
    }
    if let Some(r) = &c.service.exec_reload {
        println!("ExecReload={}", cmd_json(r));
    }
    println!("Restart={}", c.service.restart.as_str());
    println!("RestartSec={}", c.service.restart_sec);
    println!("WantedBy={}", c.install.wanted_by.as_deref().unwrap_or(""));
}

/// 打印 `cat` 结果 (pretty JSON, 不模仿 systemd unit 文件格式).
pub fn print_cat_json(c: &Unit) {
    let mut m = serde_json::Map::new();
    m.insert("Unit".to_string(), serde_json::json!(c.name.as_str()));
    m.insert(
        "Unit.Description".to_string(),
        serde_json::json!(c.description.as_str()),
    );
    m.insert(
        "Service.Type".to_string(),
        serde_json::json!(c.service.service_type.as_str()),
    );
    m.insert(
        "Service.ExecStart".to_string(),
        serde_json::json!(&c.service.exec_start),
    );
    if let Some(s) = &c.service.exec_stop {
        m.insert("Service.ExecStop".to_string(), serde_json::json!(s));
    }
    if let Some(r) = &c.service.exec_reload {
        m.insert("Service.ExecReload".to_string(), serde_json::json!(r));
    }
    m.insert(
        "Service.Restart".to_string(),
        serde_json::json!(c.service.restart.as_str()),
    );
    m.insert(
        "Service.RestartSec".to_string(),
        serde_json::json!(c.service.restart_sec),
    );
    if let Some(v) = &c.install.wanted_by {
        m.insert(
            "Install.WantedBy".to_string(),
            serde_json::json!(v.as_str()),
        );
    }
    println!("{}", serde_json::to_string_pretty(&m).unwrap_or_default());
}

/// 打印 unit 状态 查询结果, 返回 进程退出码.
///
/// init 统一返回 [`Option<SdResUnitState>`]; 具体语义由 ctl 分别处理.
/// 退出码 模仿 systemd: `is-active` 为 active=0 / 非 active=3 / 未知=4;
/// `is-failed` 为 failed=0 / 非 failed=1 / 未知=4; `is-enabled` 为
/// enabled=0 / 其它=1 / 未知=4.
pub fn print_unit_state(cmd_name: &str, state: Option<SdResUnitState>) -> u8 {
    match cmd_name {
        "is-active" => match state {
            None => {
                println!("inactive");
                4
            }
            Some(s) => {
                println!("{}", active_str(s.active));
                if matches!(
                    s.active,
                    UnitSta::Active | UnitSta::Reloading | UnitSta::Refreshing
                ) {
                    0
                } else {
                    3
                }
            }
        },
        "is-failed" => match state {
            None => {
                println!("inactive");
                4
            }
            Some(s) => {
                println!("{}", active_str(s.active));
                if s.active == UnitSta::Failed { 0 } else { 1 }
            }
        },
        "is-enabled" => match state {
            None => {
                println!("not-found");
                4
            }
            Some(s) => {
                if s.masked {
                    println!("masked");
                    1
                } else if s.enabled {
                    println!("enabled");
                    0
                } else {
                    println!("disabled");
                    1
                }
            }
        },
        _ => {
            eprintln!("unexpected unit state response");
            1
        }
    }
}

/// 打印 timer 列表.
pub fn print_timers(timers: &[SdResTimerInfo]) {
    if timers.is_empty() {
        println!("0 timers listed.");
    } else {
        for t in timers {
            println!("{}", t.name);
        }
    }
}

/// `UnitSta` -> 显示文本.
pub fn active_str(s: UnitSta) -> &'static str {
    match s {
        UnitSta::Active => "active",
        UnitSta::Reloading => "reloading",
        UnitSta::Inactive => "inactive",
        UnitSta::Failed => "failed",
        UnitSta::Activating => "activating",
        UnitSta::Deactivating => "deactivating",
        UnitSta::Maintenance => "maintenance",
        UnitSta::Refreshing => "refreshing",
    }
}

/// `UnitSubSta` -> 显示文本.
pub fn sub_str(s: UnitSubSta) -> &'static str {
    s.as_str()
}
