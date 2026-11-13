//! pm-sysd-log 程序 (CLI).
//!
//! 相当于 `journalctl` 的角色: 通过与 pm-sysd-init 交互读取日志. 同步执行.
//!
//! 初版主要实现 `-eu UNIT` 的功能: 跳到日志末尾 (`-e`) 并按 unit 过滤 (`-u`),
//! 输出格式模仿 journalctl 的 short 格式.

use std::process::ExitCode;

use chrono::{DateTime, Local};
use tokio::runtime::Runtime;

use crate::{
    cli::{complete_unit, connect, resolve_instance, set_instance},
    jsonl::{SdReq, SdReqLogQuery, SdReqLogRO, SdRes, SdResLogEntry, decode_frame, encode_frame},
    pa::{Instance, hostname},
};

mod error;

#[cfg(test)]
mod test;

pub use error::LogError;

/// 默认 最多读取条数.
const DEFAULT_LIMIT: u32 = 1000;

/// pm-sysd-log 命令行参数.
struct Args {
    /// 实例 (未显式指定时为 `None`, 连接时 再解析默认实例).
    instance: Option<Instance>,
    /// 按 unit 过滤 (`-u` / `--unit`).
    unit: Option<String>,
}

/// pm-sysd-log 命令行入口.
///
/// 由 `src/bin/pm-sysd-log.rs` 调用.
pub fn main(a: Vec<String>) -> Result<(), ExitCode> {
    if a.iter().any(|s| s == "-h" || s == "--help") {
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

    let rt = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("{}", e);
            return Err(ExitCode::FAILURE);
        }
    };

    // 解析 实际连接的实例: 与 pm-sysd-ctl 一致, 未显式指定时优先 --system,
    // 不存在则回退 --user.
    let instance = match resolve_instance(args.instance) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("{}", e);
            return Err(ExitCode::FAILURE);
        }
    };

    match rt.block_on(run(args.unit, instance)) {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("{}", e);
            Err(ExitCode::FAILURE)
        }
    }
}

/// 解析 命令行参数.
fn parse_args(a: &[String]) -> Result<Args, LogError> {
    let mut instance: Option<Instance> = None;
    let mut unit: Option<String> = None;

    let mut i = 0;
    while i < a.len() {
        let arg = &a[i];
        match arg.as_str() {
            "--system" => set_instance(&mut instance, Instance::System).map_err(LogError::Msg)?,
            "--user" => set_instance(&mut instance, Instance::User).map_err(LogError::Msg)?,
            // `-e` (跳到末尾) 是默认行为, 接受并忽略.
            "-e" | "-x" | "-xe" | "--pager-end" | "--catalog" => {}
            // `-eu UNIT` / `-euUNIT`: `-e` + `-u` (unit 过滤).
            arg if arg.starts_with("-eu") => {
                let rest = &arg[3..];
                if !rest.is_empty() {
                    unit = Some(complete_unit(rest));
                } else {
                    i += 1;
                    let v = a
                        .get(i)
                        .ok_or_else(|| LogError::Msg("-u needs UNIT".into()))?;
                    unit = Some(complete_unit(v));
                }
            }
            "-u" | "--unit" => {
                i += 1;
                let v = a
                    .get(i)
                    .ok_or_else(|| LogError::Msg("-u needs UNIT".into()))?;
                unit = Some(complete_unit(v));
            }
            "-h" | "--help" => {}
            _ => {
                if let Some(v) = arg.strip_prefix("--unit=") {
                    unit = Some(complete_unit(v));
                } else if let Some(v) = arg.strip_prefix("-u=") {
                    unit = Some(complete_unit(v));
                } else if let Some(v) = arg.strip_prefix("--machine=") {
                    set_instance(&mut instance, Instance::Machine(v.to_string()))
                        .map_err(LogError::Msg)?;
                } else if let Some(v) = arg.strip_prefix("-M=") {
                    set_instance(&mut instance, Instance::Machine(v.to_string()))
                        .map_err(LogError::Msg)?;
                } else if arg == "-M" {
                    i += 1;
                    let v = a
                        .get(i)
                        .ok_or_else(|| LogError::Msg("-M needs NAME".into()))?;
                    set_instance(&mut instance, Instance::Machine(v.clone()))
                        .map_err(LogError::Msg)?;
                } else if arg.starts_with('-') {
                    return Err(LogError::Msg(format!("unknown option: {}", arg)));
                } else {
                    return Err(LogError::Msg(format!("unexpected argument: {}", arg)));
                }
            }
        }
        i += 1;
    }

    Ok(Args { instance, unit })
}

/// 主流程.
async fn run(unit: Option<String>, instance: Instance) -> Result<(), LogError> {
    let mut stream = connect(&instance).await.map_err(LogError::Msg)?;

    // 发送日志查询命令.
    let command = SdReq::LogRO(SdReqLogRO::Query(SdReqLogQuery {
        unit,
        limit: DEFAULT_LIMIT,
        cursor: None,
    }));
    let line = encode_frame(&command).map_err(|e| LogError::Msg(e.to_string()))?;
    stream
        .send(line)
        .await
        .map_err(|e| LogError::Msg(e.to_string()))?;

    // 接收响应 并 打印.
    let line = stream
        .recv()
        .await
        .map_err(|e| LogError::Msg(e.to_string()))?;
    let resp: SdRes = decode_frame(&line).map_err(|e| LogError::Msg(e.to_string()))?;
    match resp {
        SdRes::Logs { logs, .. } => {
            print_logs(&logs);
            Ok(())
        }
        SdRes::Err(message) => Err(LogError::Msg(message)),
        _ => Err(LogError::Msg("unexpected response from init".into())),
    }
}

/// 打印日志 (模仿 journalctl 的 short 格式).
fn print_logs(logs: &[SdResLogEntry]) {
    for e in logs {
        let program = e.unit.as_deref().unwrap_or("pm-sysd-w");
        println!("{} {} {}: {}", format_ts(&e.ts), hostname(), program, e.msg);
    }
}

/// 时间戳 (UTC ISO 8601) -> 本地时间 `MMM DD HH:MM:SS`.
fn format_ts(ts: &str) -> String {
    match DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => dt
            .with_timezone(&Local)
            .format("%b %d %H:%M:%S")
            .to_string(),
        Err(_) => ts.to_string(),
    }
}

/// 打印 帮助.
fn print_help() {
    println!("pm-sysd-log: pm-sysd-w 日志查看工具 (类似 journalctl)");
    println!();
    println!("用法:");
    println!("  pm-sysd-log [选项]");
    println!();
    println!("实例选项:");
    println!("  --system        系统级实例");
    println!("  --user          当前用户实例");
    println!("  -M --machine=NAME  自定义实例");
    println!();
    println!("过滤/显示选项:");
    println!("  -e              跳到日志末尾 (默认)");
    println!("  -u --unit=UNIT  只显示指定 unit 的日志");
    println!("  -x --catalog    显示解释文本 (初版无效果)");
    println!();
    println!("  -h --help       显示帮助");
}
