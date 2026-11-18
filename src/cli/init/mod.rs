//! pm-sysd-init 程序 (CLI).
//!
//! 相当于 "PID 1" 的角色 (虽然不是真的 PID 1): 常驻进程, 管理由自己启动的
//! service, 基于 tokio 异步.
//!
//! 本模块只处理命令行紧密相关的部分 (参数解析 / 帮助).
//! 常驻主循环见 `sysd::init`, soft-reboot 见 `pa`.

use std::{env, process::ExitCode};

#[cfg(target_os = "windows")]
use std::sync::OnceLock;

use tokio::runtime::Runtime;

use crate::{
    cli::set_instance,
    conf::load_conf,
    pa::{Instance, soft_reboot},
    sysd::init::{Shutdown, run},
};

#[cfg(target_os = "windows")]
use crate::p_w::{
    PwError,
    console::adjust_console,
    service::{StopEvent, run_dispatcher},
};

#[cfg(target_os = "windows")]
static PRINT_VERSION: OnceLock<fn()> = OnceLock::new();

mod error;

#[cfg(test)]
mod test;

pub use error::InitError;

/// pm-sysd-init 启动参数
struct Args {
    /// 实例
    instance: Instance,
    /// 主配置文件路径
    conf: String,
}

/// pm-sysd-init 命令行入口.
///
/// 由 `src/bin/pm-sysd-init.rs` 调用 (参数为去掉程序名后的命令行参数).
/// `print_version` 由 bin 传入 (pm-bin 生成的版本输出函数).
pub fn main(a: Vec<String>, print_version: fn()) -> Result<(), ExitCode> {
    // Windows: 若由 SCM 启动, 进入服务模式并阻塞至服务结束
    #[cfg(target_os = "windows")]
    match run_as_service(print_version) {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(e) => {
            eprintln!("{}", e);
            return Err(ExitCode::FAILURE);
        }
    }

    run_normal(&a, print_version, true)
}

/// 常规运行流程 (命令行模式与服务模式共用).
///
/// `soft_reboot_ok` 为 true 时, `Shutdown::SoftReboot` 会重启自身.
/// 服务模式下传 false, 避免服务进程自行 fork 新进程.
fn run_normal(a: &[String], print_version: fn(), soft_reboot_ok: bool) -> Result<(), ExitCode> {
    // 控制台自适应: 非控制台启动时断开自动分配的控制台, 避免黑窗口
    #[cfg(target_os = "windows")]
    adjust_console();

    if a.iter().any(|s| s == "-h" || s == "--help") {
        print_help();
        return Ok(());
    }

    let args = match parse_args(a) {
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

    let shutdown = match rt.block_on(run(args.instance, args.conf, print_version)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}", e);
            return Err(ExitCode::FAILURE);
        }
    };

    // soft-reboot 在全部清理工作完成后用相同参数重启自身
    match shutdown {
        Shutdown::Exit => Ok(()),
        Shutdown::SoftReboot if soft_reboot_ok => match soft_reboot(a) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("{}", e);
                Err(ExitCode::FAILURE)
            }
        },
        // 服务模式下不重启自身, 按正常退出处理
        Shutdown::SoftReboot => Ok(()),
    }
}

/// Windows: 尝试作为服务运行.
///
/// 返回 `Ok(true)` 表示已作为服务运行并结束; `Ok(false)` 表示当前不是服务模式
/// (由 SCM 启动失败判定), 调用方应继续命令行流程.
#[cfg(target_os = "windows")]
fn run_as_service(print_version: fn()) -> Result<bool, PwError> {
    let _ = PRINT_VERSION.get_or_init(|| print_version);
    run_dispatcher("pm_sysd_w_user", service_body)
}

/// Windows: 服务主体回调.
///
/// 服务模式与命令行模式共用 [`run_normal`], 仅禁用 soft-reboot.
#[cfg(target_os = "windows")]
fn service_body(_stop: &'static StopEvent) -> Result<(), String> {
    let a: Vec<String> = env::args().skip(1).collect();

    let print_version = *PRINT_VERSION
        .get()
        .expect("print_version must be initialized before service dispatch");

    run_normal(&a, print_version, false).map_err(|code| format!("service run failed: {:?}", code))
}

/// pm-sysd-init --reload 命令行入口.
///
/// 只允许 `--reload` 一个参数. 从环境变量 `PM_SYSD_CONF` 读取主配置文件路径,
/// 调用 pm-tc 加载配置, 把结果 JSON 写入 stdout 后退出.
pub fn main_reload(a: &[String]) -> Result<(), ExitCode> {
    if a.len() != 1 || a[0] != "--reload" {
        eprintln!("pm-sysd-init --reload: no other arguments are allowed");
        return Err(ExitCode::FAILURE);
    }

    match run_reload() {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("{}", e);
            Err(ExitCode::FAILURE)
        }
    }
}

/// 执行 --reload 模式: 只加载 toml 配置, 不访问数据库
fn run_reload() -> Result<(), String> {
    let conf_path = env::var("PM_SYSD_CONF")
        .map_err(|_| "PM_SYSD_CONF environment variable is not set".to_string())?;
    if conf_path.is_empty() {
        return Err("PM_SYSD_CONF is empty".to_string());
    }

    let conf = load_conf(&conf_path).map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&conf).map_err(|e| e.to_string())?;
    println!("{}", json);
    Ok(())
}

/// 解析命令行参数
fn parse_args(a: &[String]) -> Result<Args, InitError> {
    let mut instance: Option<Instance> = None;
    let mut conf: Option<String> = None;

    for arg in a {
        if let Some(v) = arg.strip_prefix("--machine=") {
            check_machine(v)?;
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

/// 校验自定义实例名称 (`--machine=NAME`).
///
/// 字符集: 数字, 大小写英文字母, 下划线, 减号 (`[0-9A-Za-z_-]`).
/// 长度: 1..=100.
fn check_machine(name: &str) -> Result<(), InitError> {
    if name.is_empty() {
        return Err(InitError::Msg(
            "--machine=NAME: NAME must not be empty".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| matches!(c, '0'..='9' | 'A'..='Z' | 'a'..='z' | '_' | '-'))
    {
        return Err(InitError::Msg(
            "--machine=NAME: NAME may contain only [0-9A-Za-z_-]".into(),
        ));
    }
    if name.len() > 100 {
        return Err(InitError::Msg(
            "--machine=NAME: NAME must be at most 100 characters".into(),
        ));
    }
    Ok(())
}

/// 打印帮助
fn print_help() {
    println!("pm-sysd-init: pm-sysd-w 实例 (类似 PID 1)");
    println!();
    println!("用法:");
    println!("  pm-sysd-init --system --conf=PATH");
    println!("  pm-sysd-init --user --conf=PATH");
    println!("  pm-sysd-init --machine=NAME --conf=PATH");
    println!();
    println!("选项:");
    println!("  --system        运行系统级实例");
    println!("  --user          运行当前用户实例");
    println!("  --machine=NAME  运行自定义实例 (NAME: [0-9A-Za-z_-], 长度 1..=100)");
    println!("  --conf=PATH     主配置文件 (pm-tc toml) 路径 (必填)");
    println!("  -h --help       显示帮助");
}
