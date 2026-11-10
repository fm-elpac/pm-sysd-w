//! pm-sysd-log 程序 (CLI).
//!
//! 相当于 `journalctl` 的角色: 通过与 pm-sysd-init 交互读取日志. 同步执行.
//!
//! 初版主要实现 `-eu UNIT` 的功能: 跳到日志末尾 (`-e`) 并按 unit 过滤 (`-u`),
//! 输出格式模仿 journalctl 的 short 格式.

mod error;
#[cfg(test)]
mod test;

pub use error::LogError;

use std::process::ExitCode;

use crate::{
    jsonl::{self, Command, LogQuery, Response},
    pa::{Instance, Pa},
};

/// 默认 最多读取条数.
const DEFAULT_LIMIT: u32 = 1000;

/// pm-sysd-log 命令行参数.
struct Args {
    /// 实例.
    instance: Instance,
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

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("{}", e);
            return Err(ExitCode::FAILURE);
        }
    };

    match rt.block_on(run(args)) {
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
            "--system" => instance = Some(Instance::System),
            "--user" => instance = Some(Instance::User),
            // `-e` (跳到末尾) 是默认行为, 接受并忽略.
            "-e" | "-x" | "-xe" | "--pager-end" | "--catalog" => {}
            // `-eu UNIT` / `-euUNIT`: `-e` + `-u` (unit 过滤).
            arg if arg.starts_with("-eu") => {
                let rest = &arg[3..];
                if !rest.is_empty() {
                    unit = Some(rest.to_string());
                } else {
                    i += 1;
                    let v = a
                        .get(i)
                        .ok_or_else(|| LogError::Msg("-u needs UNIT".into()))?;
                    unit = Some(v.clone());
                }
            }
            "-u" | "--unit" => {
                i += 1;
                let v = a
                    .get(i)
                    .ok_or_else(|| LogError::Msg("-u needs UNIT".into()))?;
                unit = Some(v.clone());
            }
            "-h" | "--help" => {}
            _ => {
                if let Some(v) = arg.strip_prefix("--unit=") {
                    unit = Some(v.to_string());
                } else if let Some(v) = arg.strip_prefix("-u=") {
                    unit = Some(v.to_string());
                } else if let Some(v) = arg.strip_prefix("--machine=") {
                    instance = Some(Instance::Machine(v.to_string()));
                } else if let Some(v) = arg.strip_prefix("-M=") {
                    instance = Some(Instance::Machine(v.to_string()));
                } else if arg == "-M" {
                    i += 1;
                    let v = a
                        .get(i)
                        .ok_or_else(|| LogError::Msg("-M needs NAME".into()))?;
                    instance = Some(Instance::Machine(v.clone()));
                } else if arg.starts_with('-') {
                    return Err(LogError::Msg(format!("unknown option: {}", arg)));
                } else {
                    return Err(LogError::Msg(format!("unexpected argument: {}", arg)));
                }
            }
        }
        i += 1;
    }

    Ok(Args {
        instance: instance.unwrap_or(Instance::System),
        unit,
    })
}

/// 主流程.
async fn run(args: Args) -> Result<(), LogError> {
    let pa = crate::Platform::default();
    let transport = args.instance.transport();
    let mut stream = pa
        .ipc_connect(&transport)
        .await
        .map_err(|e| LogError::Msg(format!("connect `{}`: {}", transport, e)))?;

    // 握手.
    jsonl::send_hello(&mut stream, &jsonl::Hello::new())
        .await
        .map_err(|e| LogError::Msg(e.to_string()))?;
    let hello = jsonl::recv_hello(&mut stream)
        .await
        .map_err(|e| LogError::Msg(e.to_string()))?;
    jsonl::check_version(&hello).map_err(|e| LogError::Msg(e.to_string()))?;

    // 发送日志查询命令.
    let command = Command::Logs {
        query: LogQuery {
            unit: args.unit,
            limit: DEFAULT_LIMIT,
        },
    };
    jsonl::send_frame(&mut stream, &jsonl::Frame::Command(command))
        .await
        .map_err(|e| LogError::Msg(e.to_string()))?;

    // 接收响应 并 打印.
    match jsonl::recv_frame(&mut stream)
        .await
        .map_err(|e| LogError::Msg(e.to_string()))?
    {
        jsonl::Frame::Response(Response::Logs { logs }) => {
            print_logs(&logs);
            Ok(())
        }
        jsonl::Frame::Response(Response::Err { message }) => Err(LogError::Msg(message)),
        _ => Err(LogError::Msg("unexpected frame from init".into())),
    }
}

/// 打印日志 (模仿 journalctl 的 short 格式).
fn print_logs(logs: &[jsonl::LogEntry]) {
    for e in logs {
        let program = e.unit.as_deref().unwrap_or("pm-sysd-w");
        println!("{} {} {}: {}", format_ts(&e.ts), hostname(), program, e.msg);
    }
}

/// 时间戳 (UTC ISO 8601) -> 本地时间 `MMM DD HH:MM:SS`.
fn format_ts(ts: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%b %d %H:%M:%S")
            .to_string(),
        Err(_) => ts.to_string(),
    }
}

/// 当前主机名.
fn hostname() -> String {
    #[cfg(target_os = "linux")]
    {
        std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string())
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "localhost".to_string())
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        "localhost".to_string()
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
