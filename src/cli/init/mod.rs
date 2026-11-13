//! pm-sysd-init 程序 (CLI).
//!
//! 相当于 "PID 1" 的角色 (虽然不是真的 PID 1): 常驻进程, 管理由自己启动的
//! service, 基于 tokio 异步.
//!
//! 本模块只处理 命令行紧密相关 的部分 (参数解析 / 帮助).
//! 常驻主循环见 `sysd::init`, soft-reboot 见 `pa`.

use std::process::ExitCode;

use tokio::runtime::Runtime;

use crate::{
    cli::set_instance,
    pa::{Instance, soft_reboot},
    sysd::init::{Shutdown, run},
};

#[cfg(target_os = "windows")]
use crate::p_w::console::adjust_console;

mod error;

#[cfg(test)]
mod test;

pub use error::InitError;

/// pm-sysd-init 启动参数.
struct Args {
    /// 实例.
    instance: Instance,
    /// 主配置文件 路径.
    conf: String,
}

/// pm-sysd-init 命令行入口.
///
/// 由 `src/bin/pm-sysd-init.rs` 调用 (参数为去掉程序名后的命令行参数).
pub fn main(a: Vec<String>) -> Result<(), ExitCode> {
    // 控制台自适应: 非控制台启动时 断开 自动分配的控制台, 避免 黑窗口 (仅 Windows).
    #[cfg(target_os = "windows")]
    adjust_console();

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

    let shutdown = match rt.block_on(run(args.instance, args.conf)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", e);
            return Err(ExitCode::FAILURE);
        }
    };

    // soft-reboot 在 全部清理工作完成后 用相同参数 重启自身.
    match shutdown {
        Shutdown::Exit => Ok(()),
        Shutdown::SoftReboot => match soft_reboot(&a) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("{}", e);
                Err(ExitCode::FAILURE)
            }
        },
    }
}

/// 解析 命令行参数.
fn parse_args(a: &[String]) -> Result<Args, InitError> {
    let mut instance: Option<Instance> = None;
    let mut conf: Option<String> = None;

    for arg in a {
        if let Some(v) = arg.strip_prefix("--machine=") {
            if v.is_empty() {
                return Err(InitError::Msg(
                    "--machine=NAME: NAME must not be empty".into(),
                ));
            }
            set_instance(&mut instance, Instance::Machine(v.to_string()))
                .map_err(InitError::Msg)?;
        } else if arg == "--system" {
            set_instance(&mut instance, Instance::System).map_err(InitError::Msg)?;
        } else if arg == "--user" {
            set_instance(&mut instance, Instance::User).map_err(InitError::Msg)?;
        } else if let Some(v) = arg.strip_prefix("--conf=") {
            if v.is_empty() {
                return Err(InitError::Msg("--conf=PATH: PATH must not be empty".into()));
            }
            conf = Some(v.to_string());
        } else {
            return Err(InitError::Msg(format!("unknown argument: {}", arg)));
        }
    }

    let instance = instance
        .ok_or_else(|| InitError::Msg("missing --system / --user / --machine=XXX".into()))?;
    let conf = conf.ok_or_else(|| InitError::Msg("missing --conf=PATH".into()))?;

    Ok(Args { instance, conf })
}

/// 打印 帮助.
fn print_help() {
    println!("pm-sysd-init: pm-sysd-w 实例 (类似 PID 1)");
    println!();
    println!("用法:");
    println!("  pm-sysd-init --system --conf=PATH");
    println!("  pm-sysd-init --user --conf=PATH");
    println!("  pm-sysd-init --machine=NAME --conf=PATH");
    println!();
    println!("选项:");
    println!("  --system        运行 系统级实例");
    println!("  --user          运行 当前用户实例");
    println!("  --machine=NAME  运行 自定义实例");
    println!("  --conf=PATH     主配置文件 (pm-tc toml) 路径 (必填)");
    println!("  -h --help       显示帮助");
}
