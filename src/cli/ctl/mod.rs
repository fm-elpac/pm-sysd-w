//! pm-sysd-ctl 程序 (CLI).
//!
//! 相当于 `systemctl` 的角色: 通过与 pm-sysd-init 交互完成操作. 同步执行.
//!
//! 显示/格式化输出 由本模块负责; init 只返回 结构化 数据.

use std::process::ExitCode;

use tokio::runtime::Runtime;

use crate::{
    cli::{complete_unit, connect, resolve_instance, set_instance},
    jsonl::{SdReq, SdRes, decode_frame, encode_frame},
    pa::Instance,
};

mod error;
mod print;

#[cfg(test)]
mod test;

pub use error::CtlError;

use print::print_response;

/// pm-sysd-ctl 命令行参数 (原始, 未做命令语义检查).
struct Args {
    /// 实例 (未显式指定时为 `None`, 连接时 再解析默认实例).
    instance: Option<Instance>,
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

    // 在发送命令之前, 先做 命令语法检查 (`whoami` 不需要命令参数).
    let command = if args.command == "whoami" {
        None
    } else {
        match build_command(&args.command, &args.units, args.now) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("{}", e);
                return Err(ExitCode::FAILURE);
            }
        }
    };

    let rt = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("{}", e);
            return Err(ExitCode::FAILURE);
        }
    };

    // 解析 实际连接的实例: 未显式指定时, 优先 --system, 不存在则回退 --user.
    let instance = match resolve_instance(args.instance) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("{}", e);
            return Err(ExitCode::FAILURE);
        }
    };

    // `whoami`: 报告 实际选择的实例名称.
    if args.command == "whoami" {
        println!("{}", instance.name());
        return Ok(());
    }

    match rt.block_on(run(
        command.expect("non-whoami command"),
        &args.command,
        instance,
    )) {
        Ok(code) => {
            if code == 0 {
                Ok(())
            } else {
                Err(ExitCode::from(code))
            }
        }
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
            "--system" => set_instance(&mut instance, Instance::System).map_err(CtlError::Msg)?,
            "--user" => set_instance(&mut instance, Instance::User).map_err(CtlError::Msg)?,
            "--now" => now = true,
            "--wait" | "--no-block" => {}
            "-h" | "--help" => {}
            _ => {
                if let Some(v) = arg.strip_prefix("--machine=") {
                    set_instance(&mut instance, Instance::Machine(v.to_string()))
                        .map_err(CtlError::Msg)?;
                } else if let Some(v) = arg.strip_prefix("-M=") {
                    set_instance(&mut instance, Instance::Machine(v.to_string()))
                        .map_err(CtlError::Msg)?;
                } else if arg == "-M" {
                    i += 1;
                    let v = a
                        .get(i)
                        .ok_or_else(|| CtlError::Msg("-M needs NAME".into()))?;
                    set_instance(&mut instance, Instance::Machine(v.clone()))
                        .map_err(CtlError::Msg)?;
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

    Ok(Args {
        instance,
        command,
        units,
        now,
    })
}

/// 把 命令字符串 + 参数 转换为 结构化 [`SdReq`], 并做 语法检查.
fn build_command(command: &str, units: &[String], now: bool) -> Result<SdReq, CtlError> {
    use crate::jsonl::{
        SdReq as C, SdReqSysd as Sy, SdReqUnit as U, SdReqUnitOp as Op, SdReqUnitOpRO as Rop,
        SdReqUnitRO as Ur, SdReqUnitd as Ud, SdReqUnitdRO as Udr,
    };

    if now && !matches!(command, "enable" | "disable") {
        return Err(CtlError::Msg(
            "--now is only valid with enable/disable".into(),
        ));
    }

    // 读取 恰好 1 个 unit 参数 (名称未写后缀时 自动补 `.service`).
    let unit = |name: &str| -> Result<String, CtlError> {
        match units.len() {
            1 => Ok(complete_unit(&units[0])),
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

    // 读取 恰好 1 个 target 参数 (set-default 原样透传, 不做补全).
    let target = |name: &str| -> Result<String, CtlError> {
        match units.len() {
            1 => Ok(units[0].clone()),
            0 => Err(CtlError::Msg(format!(
                "command `{}` needs a target name",
                name
            ))),
            _ => Err(CtlError::Msg(format!(
                "command `{}` takes only 1 target name",
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
            C::UnitdRO(Udr::ListUnits)
        }
        "list-timers" => {
            no_args(command)?;
            C::UnitdRO(Udr::ListTimers)
        }
        "is-active" | "is-failed" | "is-enabled" => C::UnitRO(Ur {
            name: unit(command)?,
            op: Rop::UnitState,
        }),
        "status" => C::UnitRO(Ur {
            name: unit(command)?,
            op: Rop::Status,
        }),
        "show" | "cat" => C::UnitRO(Ur {
            name: unit(command)?,
            op: Rop::Config,
        }),
        "list-dependencies" => C::UnitRO(Ur {
            name: unit(command)?,
            op: Rop::ListDependencies,
        }),
        "start" => C::Unit(U {
            name: unit(command)?,
            op: Op::Start,
        }),
        "stop" => C::Unit(U {
            name: unit(command)?,
            op: Op::Stop,
        }),
        "reload" => C::Unit(U {
            name: unit(command)?,
            op: Op::Reload,
        }),
        "restart" => C::Unit(U {
            name: unit(command)?,
            op: Op::Restart,
        }),
        "try-restart" => C::Unit(U {
            name: unit(command)?,
            op: Op::TryRestart,
        }),
        "reload-or-restart" => C::Unit(U {
            name: unit(command)?,
            op: Op::ReloadOrRestart,
        }),
        "try-reload-or-restart" => C::Unit(U {
            name: unit(command)?,
            op: Op::TryReloadOrRestart,
        }),
        "kill" => C::Unit(U {
            name: unit(command)?,
            op: Op::Kill,
        }),
        "clean" => C::Unit(U {
            name: unit(command)?,
            op: Op::Clean,
        }),
        "reset-failed" => C::Unit(U {
            name: match units.len() {
                0 => String::new(),
                1 => complete_unit(&units[0]),
                _ => {
                    return Err(CtlError::Msg(
                        "command `reset-failed` takes at most 1 unit name".into(),
                    ));
                }
            },
            op: Op::ResetFailed,
        }),
        "enable" => C::Unit(U {
            name: unit(command)?,
            op: Op::Enable { now },
        }),
        "disable" => C::Unit(U {
            name: unit(command)?,
            op: Op::Disable { now },
        }),
        "reenable" => C::Unit(U {
            name: unit(command)?,
            op: Op::Reenable,
        }),
        "preset" => C::Unit(U {
            name: unit(command)?,
            op: Op::Preset,
        }),
        "mask" => C::Unit(U {
            name: unit(command)?,
            op: Op::Mask,
        }),
        "unmask" => C::Unit(U {
            name: unit(command)?,
            op: Op::Unmask,
        }),
        "get-default" => {
            no_args(command)?;
            C::UnitdRO(Udr::GetDefault)
        }
        "set-default" => C::UnitD(Ud::SetDefault {
            target: target(command)?,
        }),
        "show-environment" => {
            no_args(command)?;
            C::UnitdRO(Udr::ShowEnvironment)
        }
        "set-environment" => {
            no_args(command)?;
            C::UnitD(Ud::SetEnvironment)
        }
        "unset-environment" => {
            no_args(command)?;
            C::UnitD(Ud::UnsetEnvironment)
        }
        // import-environment 是 ctl 本地功能, 不直接对应一条 init 请求.
        // 实现时: 读取 ctl 自身进程 对应名称的 环境变量, 用其值调用 set-environment.
        // 目前 不实现.
        "import-environment" => {
            no_args(command)?;
            return Err(CtlError::Msg("import-environment not implemented".into()));
        }
        "daemon-reload" => {
            no_args(command)?;
            C::UnitD(Ud::DaemonReload)
        }
        "soft-reboot" => {
            no_args(command)?;
            C::SysD(Sy::SoftReboot)
        }
        "exit" => {
            no_args(command)?;
            C::SysD(Sy::Exit)
        }
        other => return Err(CtlError::Msg(format!("unknown command: {}", other))),
    })
}

/// 主流程.
async fn run(command: SdReq, command_name: &str, instance: Instance) -> Result<u8, CtlError> {
    let mut stream = connect(&instance).await.map_err(CtlError::Msg)?;

    // 发送命令.
    let line = encode_frame(&command).map_err(|e| CtlError::Msg(e.to_string()))?;
    stream
        .send(line)
        .await
        .map_err(|e| CtlError::Msg(e.to_string()))?;

    // 接收响应 并 格式化输出.
    let line = stream
        .recv()
        .await
        .map_err(|e| CtlError::Msg(e.to_string()))?;
    let resp: SdRes = decode_frame(&line).map_err(|e| CtlError::Msg(e.to_string()))?;
    Ok(print_response(command_name, &command, resp))
}

/// 格式化/打印 响应, 返回 进程退出码.

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
