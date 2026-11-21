//! pm-sysd-ctl 使用的 systemctl 风格输出.
//!
//! 输出格式与退出码尽量模仿 `systemctl` (参考
//! <https://www.freedesktop.org/software/systemd/man/latest/systemctl.html>).
//!
//! 表格命令 (`list-units`) 使用动态列宽: 每列左对齐, 列宽取该列内容最长值,
//! 列间以一个空格分隔; 首行打印大写列名, 整表左侧空两格 (对应 systemctl 的
//! 状态圆点列, 本项目省略圆点/颜色).
//!
//! 与本项目当前功能集无关或无法提供的字段会被省略, 省略清单:
//!
//! - `list-units`: 省略状态圆点 (颜色/字形) 与 JOB 列 (本项目无 job);
//!   省略 `Pass --all` 与 `list-unit-files` 提示 (对应选项/命令未实现).
//!   本项目总是列出全部已加载 unit (含 inactive), 因此只打印
//!   `N loaded units listed.`.
//! - `list-timers`: timer unit 未实现, 响应恒为空; 仍打印表头
//!   NEXT/LEFT/LAST/PASSED/UNIT/ACTIVATES, 省略数据行与 `Pass --all` 提示,
//!   只打印 `0 timers listed.`.
//! - `status`: 省略状态圆点/颜色, unit 文件路径, preset, Docs, Tasks, Memory,
//!   CPU, CGroup, 日志与进程名; 只保留 `Loaded:` / `Active:` / `Main PID:` 三行
//!   (有 pid 时打印 Main PID), 并在 `Active:` 行打印状态变化时间
//!   (`since ...; ... ago`). 省略 `Restarts` / `ExitStatus` 顶层字段 (systemd 的
//!   status 无对应顶层字段, 且本项目缺少命令/进程名).
//! - `is-enabled`: 省略 `--full` 的安装目标输出 (选项未实现), 只打印状态词;
//!   退出码按 systemd 表映射.

use chrono::{DateTime, Local, Utc};
use serde_json;

use crate::{
    conf::Unit,
    jsonl::{
        SdReq, SdReqUnitOp, SdReqUnitOpRO, SdReqUnitd, SdRes, SdResTimerInfo, SdResUnitS,
        SdResUnitState, SdResUnitStatus, SdResWhoami,
    },
};

use super::{EnableSta, UnitSta};

pub fn print_response(cmd_name: &str, cmd: &SdReq, resp: SdRes) -> u8 {
    match resp {
        SdRes::Ok(message) => {
            if let Some(m) = message {
                println!("{}", m);
            }
            if let Some(m) = action_ok_message(cmd) {
                println!("{}", m);
            }
            0
        }
        SdRes::Err(message) => {
            eprintln!("{}", message);
            if status_not_found(cmd, &message) {
                4
            } else {
                1
            }
        }
        SdRes::ListUnits(page) => {
            print_list_units(&page.list);
            0
        }
        SdRes::Status(status) => {
            print_status(&status);
            status_exit_code(status.active)
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
        SdRes::SystemState(sta) => {
            println!("{}", sta);
            sta.exit_code()
        }
        SdRes::Dependencies(page) => {
            for d in &page.list {
                println!("{}", d);
            }
            0
        }
        SdRes::Timers(page) => {
            print_timers(&page.list);
            0
        }
        SdRes::Logs(_) => {
            // ctl 不会请求日志, 协议异常
            eprintln!("unexpected response: logs");
            1
        }
        SdRes::Whoami(_) => {
            // whoami 在 ctl 中单独处理, 不会走到这里.
            eprintln!("unexpected response: whoami");
            1
        }
    }
}

/// 打印 `whoami` 结果.
///
/// 默认多行 `KEY=VALUE`; `--json` 时输出单行 JSON (使用相同 key).
pub fn print_whoami(resp: &SdRes, json: bool) -> u8 {
    match resp {
        SdRes::Whoami(w) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&whoami_json(w)).expect("encode whoami json")
                );
            } else {
                for (k, v) in whoami_pairs(w) {
                    println!("{}={}", k, v);
                }
            }
            0
        }
        SdRes::Err(message) => {
            eprintln!("{}", message);
            1
        }
        _ => {
            eprintln!("unexpected response: whoami");
            1
        }
    }
}

/// 把 `whoami` 结果转换为 JSON 对象 (key 与文本格式一致, 数字使用 JSON 数字).
fn whoami_json(w: &SdResWhoami) -> serde_json::Value {
    match w {
        SdResWhoami {
            service: Some(name),
            ..
        } => serde_json::json!({ "Service": name }),
        SdResWhoami {
            init: Some(init), ..
        } => {
            let mut m = serde_json::Map::new();
            m.insert(
                "InstanceType".to_string(),
                serde_json::json!(init.instance_type),
            );
            m.insert(
                "InstanceName".to_string(),
                serde_json::json!(init.instance_name),
            );
            m.insert("Pid".to_string(), serde_json::json!(init.pid));
            if let Some(name) = &init.service_name {
                m.insert("ServiceName".to_string(), serde_json::json!(name));
            }
            serde_json::Value::Object(m)
        }
        _ => serde_json::Value::Object(serde_json::Map::new()),
    }
}

/// 把 `whoami` 结果转换为 `(key, value)` 列表, 供文本输出使用.
fn whoami_pairs(w: &SdResWhoami) -> Vec<(String, String)> {
    if let Some(init) = &w.init {
        let mut pairs = vec![
            ("InstanceType".to_string(), init.instance_type.clone()),
            ("InstanceName".to_string(), init.instance_name.clone()),
            ("Pid".to_string(), init.pid.to_string()),
        ];
        if let Some(name) = &init.service_name {
            pairs.push(("ServiceName".to_string(), name.clone()));
        }
        pairs
    } else if let Some(service) = &w.service {
        vec![("Service".to_string(), service.clone())]
    } else {
        Vec::new()
    }
}

/// 判断错误消息是否为 status 命令的 unit 不存在 (此时返回退出码 4)
fn status_not_found(cmd: &SdReq, message: &str) -> bool {
    match cmd {
        SdReq::UnitRO(r) if matches!(r.op, SdReqUnitOpRO::Status) => {
            message == format!("Unit {} not found.", r.name)
        }
        _ => false,
    }
}

/// `status` 退出码 (模仿 systemd).
///
/// active / reloading / refreshing = 0.
/// 其它 (inactive / failed / activating / deactivating / maintenance) = 3.
fn status_exit_code(a: UnitSta) -> u8 {
    if matches!(
        a,
        UnitSta::Active | UnitSta::Reloading | UnitSta::Refreshing
    ) {
        0
    } else {
        3
    }
}

/// 操作成功时的提示消息 (大部分动作静默成功, 与 systemctl 风格一致)
pub fn action_ok_message(cmd: &SdReq) -> Option<String> {
    match cmd {
        SdReq::Unit(u) => match &u.op {
            SdReqUnitOp::Enable { .. } => Some(format!("Enabled {}.", u.name)),
            SdReqUnitOp::Disable { .. } => Some(format!("Disabled {}.", u.name)),
            SdReqUnitOp::Reenable => Some(format!("Reenabled {}.", u.name)),
            SdReqUnitOp::Mask { .. } => Some(format!("Masked {}.", u.name)),
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

/// 打印 `list-units` 结果 (模仿 systemctl 表格)
pub fn print_list_units(units: &[SdResUnitState]) {
    let headers = ["UNIT", "LOAD", "ACTIVE", "SUB", "DESCRIPTION"];
    let rows: Vec<Vec<String>> = units
        .iter()
        .map(|u| {
            vec![
                u.name.clone(),
                u.load.to_string(),
                u.active.as_str().to_string(),
                u.sub.as_str().to_string(),
                u.description.clone(),
            ]
        })
        .collect();

    print_table(&headers, &rows, 2);

    if !units.is_empty() {
        println!();
        println!(
            "{:<6} = Reflects whether the unit definition was properly loaded.",
            "LOAD"
        );
        println!(
            "{:<6} = The high-level unit activation state, i.e. generalization of SUB.",
            "ACTIVE"
        );
        println!(
            "{:<6} = The low-level unit activation state, values depend on unit type.",
            "SUB"
        );
    }

    println!();
    println!("{} loaded units listed.", units.len());
}

/// 打印 systemctl 风格表格: 每列左对齐, 列宽取该列最长内容, 列间一个空格.
///
/// `indent` 为整表左侧缩进空格数 (`list-units` 用 2, 对应省略的状态圆点列).
fn print_table(headers: &[&str], rows: &[Vec<String>], indent: usize) {
    let cols = headers.len();
    let mut widths = vec![0usize; cols];
    for (i, h) in headers.iter().enumerate() {
        widths[i] = widths[i].max(h.len());
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell_display(cell).len());
        }
    }

    let print_row = |cells: &[String]| {
        let mut line = " ".repeat(indent);
        for (i, cell) in cells.iter().enumerate() {
            let text = cell_display(cell);
            if i + 1 == cols {
                line.push_str(text);
            } else {
                line.push_str(&format!("{:<width$}", text, width = widths[i]));
                line.push(' ');
            }
        }
        println!("{}", line);
    };

    let header_cells: Vec<String> = headers.iter().map(|s| s.to_string()).collect();
    print_row(&header_cells);
    for row in rows {
        print_row(row);
    }
}

/// 空单元格显示为 `-` (模仿 systemd 的 ersatz string)
fn cell_display(cell: &str) -> &str {
    if cell.is_empty() { "-" } else { cell }
}

/// 打印 `status` 结果
pub fn print_status(s: &SdResUnitStatus) {
    if !s.description.is_empty() && s.description != s.name {
        println!("{} - {}", s.name, s.description);
    } else {
        println!("{}", s.name);
    }
    println!("{:>11}: {} ({})", "Loaded", s.load, s.enable);
    match s.since {
        Some(ms) => println!(
            "{:>11}: {} ({}){}",
            "Active",
            s.active.as_str(),
            s.sub.as_str(),
            format_status_since(ms)
        ),
        None => println!(
            "{:>11}: {} ({})",
            "Active",
            s.active.as_str(),
            s.sub.as_str()
        ),
    }

    match &s.s {
        SdResUnitS::Service(ss) => {
            if let Some(pid) = ss.pid {
                println!("{:>11}: {}", "Main PID", pid);
            }
        }
        SdResUnitS::Timer(_) => {}
    }
}

/// 把 Unix 毫秒时间戳格式化为 `status` 的 `Active:` 行 `since` 部分.
///
/// 绝对时间模仿 systemd 的默认格式 (`%a %Y-%m-%d %H:%M:%S`, 本地时区),
/// 相对时间见 [`format_relative`].
fn format_status_since(ms: i64) -> String {
    let Some(dt) = DateTime::<Utc>::from_timestamp_millis(ms) else {
        return String::new();
    };
    let abs = dt.with_timezone(&Local).format("%a %Y-%m-%d %H:%M:%S");
    format!(" since {}; {}", abs, format_relative(ms))
}

const USEC_PER_MSEC: u64 = 1_000;
const USEC_PER_SEC: u64 = 1_000_000;
const USEC_PER_MINUTE: u64 = 60 * USEC_PER_SEC;
const USEC_PER_HOUR: u64 = 60 * USEC_PER_MINUTE;
const USEC_PER_DAY: u64 = 24 * USEC_PER_HOUR;
const USEC_PER_WEEK: u64 = 7 * USEC_PER_DAY;
const USEC_PER_YEAR: u64 = USEC_PER_DAY * 3652425 / 10000;
const USEC_PER_MONTH: u64 = USEC_PER_YEAR / 12;

/// 把 Unix 毫秒时间戳格式化为 `... ago` 形式 (模仿 systemd 的
/// `format_timestamp_relative`).
fn format_relative(ms: i64) -> String {
    let now_ms = Utc::now().timestamp_millis();
    let d = ((now_ms - ms).max(0) as u64) * USEC_PER_MSEC;

    let unit = |n: u64, one: &str, many: &str| -> String {
        format!("{} {}", n, if n == 1 { one } else { many })
    };

    if d >= USEC_PER_YEAR {
        return format!(
            "{} {} ago",
            unit(d / USEC_PER_YEAR, "year", "years"),
            unit((d % USEC_PER_YEAR) / USEC_PER_MONTH, "month", "months"),
        );
    }
    if d >= USEC_PER_MONTH {
        return format!(
            "{} {} ago",
            unit(d / USEC_PER_MONTH, "month", "months"),
            unit((d % USEC_PER_MONTH) / USEC_PER_DAY, "day", "days"),
        );
    }
    if d >= USEC_PER_WEEK {
        return format!(
            "{} {} ago",
            unit(d / USEC_PER_WEEK, "week", "weeks"),
            unit((d % USEC_PER_WEEK) / USEC_PER_DAY, "day", "days"),
        );
    }
    if d >= 2 * USEC_PER_DAY {
        return format!("{} ago", unit(d / USEC_PER_DAY, "day", "days"));
    }
    if d >= 25 * USEC_PER_HOUR {
        return format!("1 day {}h ago", (d - USEC_PER_DAY) / USEC_PER_HOUR);
    }
    if d >= 6 * USEC_PER_HOUR {
        return format!("{}h ago", d / USEC_PER_HOUR);
    }
    if d >= USEC_PER_HOUR {
        return format!(
            "{}h {}min ago",
            d / USEC_PER_HOUR,
            (d % USEC_PER_HOUR) / USEC_PER_MINUTE,
        );
    }
    if d >= 5 * USEC_PER_MINUTE {
        return format!("{}min ago", d / USEC_PER_MINUTE);
    }
    if d >= USEC_PER_MINUTE {
        return format!(
            "{}min {}s ago",
            d / USEC_PER_MINUTE,
            (d % USEC_PER_MINUTE) / USEC_PER_SEC,
        );
    }
    if d >= USEC_PER_SEC {
        return format!("{}s ago", d / USEC_PER_SEC);
    }
    if d >= USEC_PER_MSEC {
        return format!("{}ms ago", d / USEC_PER_MSEC);
    }
    if d > 0 {
        return format!("{}us ago", d);
    }
    "now".to_string()
}

/// 打印 unit 配置查询结果
pub fn print_config(cmd_name: &str, c: &Unit) {
    match cmd_name {
        "cat" => print_cat_json(c),
        _ => print_show(c),
    }
}

/// 把 unit 配置转换为 `(key, value)` 列表, 供 `show` / `cat` 共用.
///
/// key 使用配置文件中的完整 key (例如 `Service.ExecStart`), value 使用
/// JSON 值, 保证两个命令输出相同的数据.
fn config_pairs(c: &Unit) -> Vec<(String, serde_json::Value)> {
    let environment = c
        .service
        .environment
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>();

    vec![
        ("Unit".to_string(), serde_json::json!(c.name.as_str())),
        (
            "Unit.Description".to_string(),
            serde_json::json!(c.description.as_str()),
        ),
        (
            "Unit.StartLimitIntervalSec".to_string(),
            serde_json::json!(c.start_limit_interval_sec),
        ),
        (
            "Unit.StartLimitBurst".to_string(),
            serde_json::json!(c.start_limit_burst),
        ),
        (
            "Service.Type".to_string(),
            serde_json::json!(c.service.service_type.as_str()),
        ),
        (
            "Service.RemainAfterExit".to_string(),
            serde_json::json!(c.service.remain_after_exit),
        ),
        (
            "Service.ExecStart".to_string(),
            serde_json::json!(&c.service.exec_start),
        ),
        (
            "Service.ExecStop".to_string(),
            serde_json::json!(&c.service.exec_stop),
        ),
        (
            "Service.ExecReload".to_string(),
            serde_json::json!(&c.service.exec_reload),
        ),
        (
            "Service.Restart".to_string(),
            serde_json::json!(c.service.restart.as_str()),
        ),
        (
            "Service.RestartSec".to_string(),
            serde_json::json!(c.service.restart_sec),
        ),
        (
            "Service.TimeoutStartSec".to_string(),
            serde_json::json!(c.service.timeout_start_sec),
        ),
        (
            "Service.TimeoutStopSec".to_string(),
            serde_json::json!(c.service.timeout_stop_sec),
        ),
        (
            "Service.WorkingDirectory".to_string(),
            serde_json::json!(c.service.working_directory.clone().unwrap_or_default()),
        ),
        (
            "Service.Environment".to_string(),
            serde_json::json!(environment),
        ),
        (
            "Install.WantedBy".to_string(),
            serde_json::json!(&c.install.wanted_by),
        ),
        (
            "Win.CreateNoWindow".to_string(),
            serde_json::json!(c.win.create_no_window),
        ),
    ]
}

/// 打印 `show` 结果 (`key=value` 格式, key 与配置文件一致)
pub fn print_show(c: &Unit) {
    for (key, value) in config_pairs(c) {
        println!("{}={}", key, show_value(&value));
    }
}

/// 把 JSON 值转换为 `show` 的单行文本值.
///
/// 字符串原样输出, 数字输出十进制, 布尔使用 systemd 风格 `yes` / `no`,
/// 数组/对象输出紧凑 JSON.
fn show_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => {
            if *b {
                "yes".to_string()
            } else {
                "no".to_string()
            }
        }
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            serde_json::to_string(v).unwrap_or_default()
        }
        serde_json::Value::Null => String::new(),
    }
}

/// 打印 `cat` 结果 (pretty JSON, 不模仿 systemd unit 文件格式)
pub fn print_cat_json(c: &Unit) {
    let m: serde_json::Map<String, serde_json::Value> = config_pairs(c).into_iter().collect();
    println!("{}", serde_json::to_string_pretty(&m).unwrap_or_default());
}

/// 打印 unit 状态查询结果, 返回进程退出码.
///
/// 退出码模仿 systemd: `is-active` 为 active=0 / 非 active=3 / 未知=4.
/// `is-failed` 为 failed=0 / 非 failed=1 / 未知=4; `is-enabled` 为
/// enabled 等=0 / linked, masked, disabled, bad=1 / 未知=4.
pub fn print_unit_state(cmd_name: &str, state: Option<SdResUnitState>) -> u8 {
    match cmd_name {
        "is-active" => match state {
            None => {
                println!("inactive");
                4
            }
            Some(s) => {
                println!("{}", s.active.as_str());
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
                println!("{}", s.active.as_str());
                if s.active == UnitSta::Failed { 0 } else { 1 }
            }
        },
        "is-enabled" => match state {
            None => {
                println!("not-found");
                4
            }
            Some(s) => {
                println!("{}", s.enable);
                enable_exit_code(s.enable)
            }
        },
        _ => {
            eprintln!("unexpected unit state response");
            1
        }
    }
}

/// `is-enabled` 退出码 (模仿 systemd).
///
/// 0: enabled / enabled-runtime / alias / static / indirect / generated / transient.
/// 1: linked / linked-runtime / masked / masked-runtime / disabled / bad.
fn enable_exit_code(e: EnableSta) -> u8 {
    match e {
        EnableSta::Enabled
        | EnableSta::EnabledRuntime
        | EnableSta::Alias
        | EnableSta::Static
        | EnableSta::Indirect
        | EnableSta::Generated
        | EnableSta::Transient => 0,
        EnableSta::Linked
        | EnableSta::LinkedRuntime
        | EnableSta::Masked
        | EnableSta::MaskedRuntime
        | EnableSta::Disabled
        | EnableSta::Bad => 1,
    }
}

/// 打印 `list-timers` 结果.
///
/// timer unit 未实现, init 恒返回空列表. 仍打印 systemd 风格表头
/// (NEXT/LEFT/LAST/PASSED/UNIT/ACTIVATES), 之后打印计数.
pub fn print_timers(timers: &[SdResTimerInfo]) {
    let headers = ["NEXT", "LEFT", "LAST", "PASSED", "UNIT", "ACTIVATES"];
    let rows: Vec<Vec<String>> = timers
        .iter()
        .map(|t| {
            vec![
                // NEXT / LEFT / LAST / PASSED / ACTIVATES 数据未实现, 用 `-` 占位.
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                "-".to_string(),
                t.name.clone(),
                "-".to_string(),
            ]
        })
        .collect();

    print_table(&headers, &rows, 0);
    println!();
    println!("{} timers listed.", timers.len());
}

#[cfg(test)]
mod test;
