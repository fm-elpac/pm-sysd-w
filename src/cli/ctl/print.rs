//! ctl 命令响应的格式化/打印.
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
//! - `status`: 省略状态圆点/颜色, 时间戳 (`since ... ago`), unit 文件路径,
//!   preset, Docs, Tasks, Memory, CPU, CGroup, 日志与进程名; 只保留
//!   `Loaded:` / `Active:` / `Main PID:` 三行 (有 pid 时打印 Main PID).
//!   省略 `Restarts` / `ExitStatus` 顶层字段 (systemd 的 status 无对应顶层
//!   字段, 且本项目缺少命令/进程名/时间戳, 无法按 systemd 格式呈现).
//! - `is-enabled`: 省略 `--full` 的安装目标输出 (选项未实现), 只打印状态词;
//!   退出码按 systemd 表映射.

use serde_json;

use crate::{
    conf::Unit,
    jsonl::{
        SdReq, SdReqUnitOp, SdReqUnitOpRO, SdReqUnitd, SdRes, SdResTimerInfo, SdResUnitS,
        SdResUnitState, SdResUnitStatus,
    },
    sysd::{EnableSta, UnitSta},
};

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
        SdRes::ListUnits(units) => {
            print_list_units(&units);
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

/// 判断错误消息是否为 status 命令的 unit 不存在 (此时返回退出码 4).
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
/// active / reloading / refreshing = 0; 其它 (inactive / failed / activating /
/// deactivating / maintenance) = 3.
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

/// 操作成功时 的 提示消息 (大部分动作 静默成功, 与 systemctl 风格一致).
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

/// 打印 `list-units` 结果 (模仿 systemctl 表格).
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

/// 空单元格显示为 `-` (模仿 systemd 的 ersatz string).
fn cell_display(cell: &str) -> &str {
    if cell.is_empty() { "-" } else { cell }
}

/// 打印 `status` 结果.
pub fn print_status(s: &SdResUnitStatus) {
    if !s.description.is_empty() && s.description != s.name {
        println!("{} - {}", s.name, s.description);
    } else {
        println!("{}", s.name);
    }
    println!("{:>11}: {} ({})", "Loaded", s.load, s.enable);
    println!(
        "{:>11}: {} ({})",
        "Active",
        s.active.as_str(),
        s.sub.as_str()
    );

    match &s.s {
        SdResUnitS::Service(ss) => {
            if let Some(pid) = ss.pid {
                println!("{:>11}: {}", "Main PID", pid);
            }
        }
        SdResUnitS::Timer(_) => {}
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
/// 退出码 模仿 systemd: `is-active` 为 active=0 / 非 active=3 / 未知=4;
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
/// 0: enabled / enabled-runtime / alias / static / indirect / generated / transient;
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
/// timer unit 未实现, init 恒返回空列表; 仍打印 systemd 风格表头
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
