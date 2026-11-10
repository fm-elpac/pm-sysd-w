//! pm-sysd-ctl 程序 (CLI).
//!
//! 相当于 `systemctl` 的角色: 通过与 pm-sysd-init 交互完成操作. 同步执行.
//!
//! 显示/格式化输出 由本模块负责; init 只返回 结构化 数据.

mod error;
#[cfg(test)]
mod test;

pub use error::CtlError;

use std::process::ExitCode;

use crate::{
    jsonl::{
        self, ActiveState, Command, EnableState, Response, SubState, TimerInfo, UnitConfig,
        UnitInfo, UnitStatus,
    },
    pa::{Instance, Pa},
};

/// pm-sysd-ctl 命令行参数 (原始, 未做命令语义检查).
struct Args {
    /// 实例.
    instance: Instance,
    /// 命令 (字符串).
    command: String,
    /// unit 位置参数.
    units: Vec<String>,
    /// `--now` (用于 `enable --now` / `disable --now`).
    now: bool,
}

/// pm-sysd-ctl 命令行入口.
///
/// 由 `src/bin/pm-sysd-ctl.rs` 调用.
pub fn main(a: Vec<String>) -> Result<(), ExitCode> {
    if a.is_empty() || a.iter().any(|s| s == "-h" || s == "--help") {
        print_help();
        return Ok(());
    }

    let args = match parse_args(&a) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{}", e);
            return Err(ExitCode::FAILURE);
        }
    };

    // `whoami`: 报告 实例名称 (无需连接).
    if args.command == "whoami" {
        println!("{}", args.instance.name());
        return Ok(());
    }

    // 在发送命令之前, 先做 命令语法检查.
    let command = match build_command(&args.command, &args.units, args.now) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", e);
            return Err(ExitCode::FAILURE);
        }
    };

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("{}", e);
            return Err(ExitCode::FAILURE);
        }
    };

    match rt.block_on(run(command, args.instance)) {
        Ok(true) => Ok(()),
        Ok(false) => Err(ExitCode::FAILURE),
        Err(e) => {
            eprintln!("{}", e);
            Err(ExitCode::FAILURE)
        }
    }
}

/// 解析 命令行参数 (不含 命令语义检查).
fn parse_args(a: &[String]) -> Result<Args, CtlError> {
    let mut instance: Option<Instance> = None;
    let mut command: Option<String> = None;
    let mut units = Vec::new();
    let mut now = false;

    let mut i = 0;
    while i < a.len() {
        let arg = &a[i];
        match arg.as_str() {
            "--system" => instance = Some(Instance::System),
            "--user" => instance = Some(Instance::User),
            "--now" => now = true,
            "--wait" | "--no-block" => {}
            "-h" | "--help" => {}
            _ => {
                if let Some(v) = arg.strip_prefix("--machine=") {
                    instance = Some(Instance::Machine(v.to_string()));
                } else if let Some(v) = arg.strip_prefix("-M=") {
                    instance = Some(Instance::Machine(v.to_string()));
                } else if arg == "-M" {
                    i += 1;
                    let v = a
                        .get(i)
                        .ok_or_else(|| CtlError::Msg("-M needs NAME".into()))?;
                    instance = Some(Instance::Machine(v.clone()));
                } else if arg.starts_with('-') {
                    return Err(CtlError::Msg(format!("unknown option: {}", arg)));
                } else if command.is_none() {
                    command = Some(arg.clone());
                } else {
                    units.push(arg.clone());
                }
            }
        }
        i += 1;
    }

    let command = command.ok_or_else(|| CtlError::Msg("missing command".into()))?;
    let instance = instance.unwrap_or(Instance::System);

    Ok(Args {
        instance,
        command,
        units,
        now,
    })
}

/// 把 命令字符串 + 参数 转换为 结构化 [`Command`], 并做 语法检查.
fn build_command(command: &str, units: &[String], now: bool) -> Result<Command, CtlError> {
    use Command as C;

    if now && !matches!(command, "enable" | "disable") {
        return Err(CtlError::Msg(
            "--now is only valid with enable/disable".into(),
        ));
    }

    // 读取 恰好 1 个 unit 参数.
    let unit = |name: &str| -> Result<String, CtlError> {
        match units.len() {
            1 => Ok(units[0].clone()),
            0 => Err(CtlError::Msg(format!(
                "command `{}` needs a unit name",
                name
            ))),
            _ => Err(CtlError::Msg(format!(
                "command `{}` takes only 1 unit name",
                name
            ))),
        }
    };

    // 要求 0 个参数.
    let no_args = |name: &str| -> Result<(), CtlError> {
        if units.is_empty() {
            Ok(())
        } else {
            Err(CtlError::Msg(format!(
                "command `{}` takes no arguments",
                name
            )))
        }
    };

    Ok(match command {
        "list-units" => {
            no_args(command)?;
            C::ListUnits
        }
        "list-timers" => {
            no_args(command)?;
            C::ListTimers
        }
        "is-active" => C::IsActive {
            unit: unit(command)?,
        },
        "is-failed" => C::IsFailed {
            unit: unit(command)?,
        },
        "status" => C::Status {
            unit: unit(command)?,
        },
        "show" => C::Show {
            unit: unit(command)?,
        },
        "cat" => C::Cat {
            unit: unit(command)?,
        },
        "list-dependencies" => C::ListDependencies {
            unit: unit(command)?,
        },
        "start" => C::Start {
            unit: unit(command)?,
        },
        "stop" => C::Stop {
            unit: unit(command)?,
        },
        "reload" => C::Reload {
            unit: unit(command)?,
        },
        "restart" => C::Restart {
            unit: unit(command)?,
        },
        "try-restart" => C::TryRestart {
            unit: unit(command)?,
        },
        "reload-or-restart" => C::ReloadOrRestart {
            unit: unit(command)?,
        },
        "try-reload-or-restart" => C::TryReloadOrRestart {
            unit: unit(command)?,
        },
        "kill" => C::Kill {
            unit: unit(command)?,
        },
        "clean" => C::Clean {
            unit: unit(command)?,
        },
        "reset-failed" => C::ResetFailed {
            unit: match units.len() {
                0 => None,
                1 => Some(units[0].clone()),
                _ => {
                    return Err(CtlError::Msg(
                        "command `reset-failed` takes at most 1 unit name".into(),
                    ));
                }
            },
        },
        "enable" => C::Enable {
            unit: unit(command)?,
            now,
        },
        "disable" => C::Disable {
            unit: unit(command)?,
            now,
        },
        "reenable" => C::Reenable {
            unit: unit(command)?,
        },
        "preset" => C::Preset {
            unit: unit(command)?,
        },
        "is-enabled" => C::IsEnabled {
            unit: unit(command)?,
        },
        "mask" => C::Mask {
            unit: unit(command)?,
        },
        "unmask" => C::Unmask {
            unit: unit(command)?,
        },
        "get-default" => {
            no_args(command)?;
            C::GetDefault
        }
        "set-default" => C::SetDefault {
            target: unit(command)?,
        },
        "show-environment" => {
            no_args(command)?;
            C::ShowEnvironment
        }
        "set-environment" => {
            no_args(command)?;
            C::SetEnvironment
        }
        "unset-environment" => {
            no_args(command)?;
            C::UnsetEnvironment
        }
        "import-environment" => {
            no_args(command)?;
            C::ImportEnvironment
        }
        "daemon-reload" => {
            no_args(command)?;
            C::DaemonReload
        }
        "soft-reboot" => {
            no_args(command)?;
            C::SoftReboot
        }
        "exit" => {
            no_args(command)?;
            C::Exit
        }
        other => return Err(CtlError::Msg(format!("unknown command: {}", other))),
    })
}

/// 主流程.
async fn run(command: Command, instance: Instance) -> Result<bool, CtlError> {
    // 平台 实现 (编译时确定 具体类型).
    let pa = crate::Platform::default();
    let transport = instance.transport();
    let mut stream = pa
        .ipc_connect(&transport)
        .await
        .map_err(|e| CtlError::Msg(format!("connect `{}`: {}", transport, e)))?;

    // 握手.
    jsonl::send_hello(&mut stream, &jsonl::Hello::new())
        .await
        .map_err(|e| CtlError::Msg(e.to_string()))?;
    let hello = jsonl::recv_hello(&mut stream)
        .await
        .map_err(|e| CtlError::Msg(e.to_string()))?;
    jsonl::check_version(&hello).map_err(|e| CtlError::Msg(e.to_string()))?;

    // 发送命令.
    jsonl::send_frame(&mut stream, &jsonl::Frame::Command(command.clone()))
        .await
        .map_err(|e| CtlError::Msg(e.to_string()))?;

    // 接收响应 并 格式化输出.
    match jsonl::recv_frame(&mut stream)
        .await
        .map_err(|e| CtlError::Msg(e.to_string()))?
    {
        jsonl::Frame::Response(resp) => Ok(print_response(&command, resp)),
        _ => Err(CtlError::Msg("unexpected frame from init".into())),
    }
}

/// 格式化/打印 响应, 返回 是否成功.
fn print_response(cmd: &Command, resp: Response) -> bool {
    match resp {
        Response::Ok => {
            if let Some(m) = action_ok_message(cmd) {
                println!("{}", m);
            }
            true
        }
        Response::Err { message } => {
            eprintln!("{}", message);
            false
        }
        Response::ListUnits { units } => {
            print_list_units(&units);
            true
        }
        Response::Status { status } => {
            print_status(&status);
            true
        }
        Response::Config { config } => {
            print_config(cmd, &config);
            true
        }
        Response::Bool { value } => {
            print_bool(cmd, value);
            true
        }
        Response::EnableState { state } => {
            println!("{}", enable_state_str(state));
            true
        }
        Response::DefaultTarget { target } => {
            println!("{}", target);
            true
        }
        Response::Dependencies { deps } => {
            for d in &deps {
                println!("{}", d);
            }
            true
        }
        Response::Timers { timers } => {
            print_timers(&timers);
            true
        }
        Response::Logs { .. } => {
            // ctl 不会请求日志; 视为协议异常.
            eprintln!("unexpected response: logs");
            false
        }
    }
}

/// 操作成功时 的 提示消息 (大部分动作 静默成功, 与 systemctl 风格一致).
fn action_ok_message(cmd: &Command) -> Option<String> {
    use Command as C;
    match cmd {
        C::Enable { unit, .. } => Some(format!("Enabled {}.", unit)),
        C::Disable { unit, .. } => Some(format!("Disabled {}.", unit)),
        C::Reenable { unit } => Some(format!("Reenabled {}.", unit)),
        C::Mask { unit } => Some(format!("Masked {}.", unit)),
        C::Unmask { unit } => Some(format!("Unmasked {}.", unit)),
        C::SetDefault { target } => Some(format!("Default target set to {}.", target)),
        C::DaemonReload => Some("Reloaded configuration.".to_string()),
        C::ResetFailed { unit: Some(u) } => Some(format!("Reset failed state of {}.", u)),
        C::ResetFailed { unit: None } => Some("Reset all failed states.".to_string()),
        _ => None,
    }
}

/// 打印 `list-units` 结果.
fn print_list_units(units: &[UnitInfo]) {
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
fn print_status(s: &UnitStatus) {
    println!("{} - {}", s.name, s.description);
    let load = if s.masked {
        "masked"
    } else if s.enabled {
        "enabled"
    } else {
        "disabled"
    };
    println!("   Loaded: {} ({})", "loaded", load);
    match s.pid {
        Some(pid) => println!(
            "   Active: {} ({}) (pid {})",
            active_str(s.active),
            sub_str(s.sub),
            pid
        ),
        None => println!("   Active: {} ({})", active_str(s.active), sub_str(s.sub)),
    }
    if s.restart_count > 0 {
        println!("   Restarts: {}", s.restart_count);
    }
    if let Some(code) = s.exit_status {
        println!("   ExitStatus: {}", code);
    }
}

/// 打印 `show` / `cat` 结果.
fn print_config(cmd: &Command, c: &UnitConfig) {
    match cmd {
        Command::Show { .. } => {
            println!("Name={}", c.name);
            println!("Description={}", c.description);
            println!("Type={}", c.service_type);
            println!("ExecStart={}", c.exec_start);
            if let Some(s) = &c.exec_stop {
                println!("ExecStop={}", s);
            }
            if let Some(r) = &c.exec_reload {
                println!("ExecReload={}", r);
            }
            println!("Restart={}", c.restart);
            println!("RestartSec={}", c.restart_sec);
            println!("WantedBy={}", c.wanted_by.as_deref().unwrap_or(""));
        }
        _ => {
            println!("[Unit]");
            println!("Description={}", c.description);
            println!("[Service]");
            println!("ExecStart={}", c.exec_start);
            if let Some(s) = &c.exec_stop {
                println!("ExecStop={}", s);
            }
            if let Some(r) = &c.exec_reload {
                println!("ExecReload={}", r);
            }
            println!("Restart={}", c.restart);
            println!("RestartSec={}", c.restart_sec);
        }
    }
}

/// 打印 布尔结果 (is-active / is-failed).
fn print_bool(cmd: &Command, value: bool) {
    match cmd {
        Command::IsActive { .. } => println!("{}", if value { "active" } else { "inactive" }),
        Command::IsFailed { .. } => println!("{}", if value { "failed" } else { "inactive" }),
        _ => println!("{}", value),
    }
}

/// 打印 timer 列表.
fn print_timers(timers: &[TimerInfo]) {
    if timers.is_empty() {
        println!("0 timers listed.");
    } else {
        for t in timers {
            println!("{}", t.name);
        }
    }
}

/// `ActiveState` -> 显示文本.
fn active_str(s: ActiveState) -> &'static str {
    match s {
        ActiveState::Active => "active",
        ActiveState::Inactive => "inactive",
        ActiveState::Failed => "failed",
    }
}

/// `SubState` -> 显示文本.
fn sub_str(s: SubState) -> &'static str {
    match s {
        SubState::Running => "running",
        SubState::Exited => "exited",
        SubState::Dead => "dead",
        SubState::Failed => "failed",
    }
}

/// `EnableState` -> 显示文本.
fn enable_state_str(s: EnableState) -> &'static str {
    match s {
        EnableState::Enabled => "enabled",
        EnableState::Disabled => "disabled",
        EnableState::Masked => "masked",
    }
}

/// 打印 帮助.
fn print_help() {
    println!("pm-sysd-ctl: pm-sysd-w 命令行工具 (类似 systemctl)");
    println!();
    println!("用法:");
    println!("  pm-sysd-ctl [选项] <命令> [UNIT ...]");
    println!();
    println!("实例选项:");
    println!("  --system        系统级实例");
    println!("  --user          当前用户实例");
    println!("  -M --machine=NAME  自定义实例");
    println!();
    println!("常用命令:");
    println!("  list-units  start  stop  restart  status  enable  disable  is-enabled");
    println!("  mask  unmask  get-default  set-default  daemon-reload  exit");
    println!();
    println!("选项:");
    println!("  --now           enable/disable 同时 start/stop");
    println!("  --wait          等待操作完成");
    println!("  --no-block      不等待 (默认)");
    println!("  -h --help       显示帮助");
}
