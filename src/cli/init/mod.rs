//! pm-sysd-init 程序 (CLI).
//!
//! 相当于 "PID 1" 的角色 (虽然不是真的 PID 1): 常驻进程, 管理由自己启动的
//! service, 基于 tokio 异步.

mod error;
#[cfg(test)]
mod test;

pub use error::InitError;

use std::{process::ExitCode, sync::Arc};

use crate::{
    conf, db, jsonl,
    pa::{Instance, Pa},
    sysd::Sysd,
};

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
    // 控制台 自适应: 非 控制台 启动时 断开 自动分配的控制台, 避免 黑窗口 (仅 Windows).
    #[cfg(target_os = "windows")]
    crate::p_w::console::adjust_console();

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
            instance = Some(Instance::Machine(v.to_string()));
        } else if arg == "--system" {
            instance = Some(Instance::System);
        } else if arg == "--user" {
            instance = Some(Instance::User);
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

/// 主流程.
async fn run(args: Args) -> Result<(), InitError> {
    // 加载 配置 (units + 数据库 路径).
    let conf = conf::load_conf(&args.conf).map_err(|e| InitError::Msg(e.to_string()))?;

    // 打开 数据库.
    let db = Arc::new(db::Db::open(&conf.db_path).map_err(|e| InitError::Msg(e.to_string()))?);

    // 平台 实现 (编译时确定 具体类型).
    let pa = Arc::new(crate::Platform::default());

    // 创建 服务管理器 并 启动 监控任务.
    let sysd = Arc::new(Sysd::new(pa.clone(), db, &args.conf, conf.units));
    sysd.spawn_monitor();

    // 启动 默认 target 中 enable 的 service.
    if let Err(e) = sysd.start_default_target().await {
        log::error!("start default target: {}", e);
    }

    // 绑定 IPC 监听 (同时做 实例唯一性检测).
    let transport = args.instance.transport();
    let mut listener = pa.ipc_listen(&transport).map_err(|e| {
        InitError::Msg(format!(
            "instance `{}` ({}): {}",
            args.instance.name(),
            transport,
            e
        ))
    })?;

    log::info!(
        "pm-sysd-init `{}` listening on `{}`",
        args.instance.name(),
        transport
    );
    sysd.log("info", None, "pm-sysd-init started");

    // 退出信号 (处理 `exit` / `soft-reboot` 后 通知主循环).
    let (exit_tx, mut exit_rx) = tokio::sync::mpsc::channel::<()>(1);

    loop {
        tokio::select! {
            _ = exit_rx.recv() => {
                break;
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok(stream) => {
                        let sysd = sysd.clone();
                        let exit_tx = exit_tx.clone();
                        tokio::spawn(async move {
                            handle_conn(stream, sysd, exit_tx).await;
                        });
                    }
                    Err(e) => {
                        log::error!("accept: {}", e);
                    }
                }
            }
        }
    }

    // 优雅退出: 记录日志, 停止 所有 service, 等待日志落库.
    sysd.log("info", None, "pm-sysd-init exiting");
    let _ = sysd.stop_all().await;
    let _ = sysd.flush_logs();
    Ok(())
}

/// 处理 单个 IPC 连接.
async fn handle_conn(
    mut stream: crate::pa::BoxStream,
    sysd: Arc<Sysd<crate::Platform>>,
    exit_tx: tokio::sync::mpsc::Sender<()>,
) {
    // 握手.
    let hello = match jsonl::recv_hello(&mut stream).await {
        Ok(h) => h,
        Err(e) => {
            log::warn!("handshake recv: {}", e);
            return;
        }
    };
    if let Err(e) = jsonl::check_version(&hello) {
        log::warn!("handshake version: {}", e);
        return;
    }
    if let Err(e) = jsonl::send_hello(&mut stream, &jsonl::Hello::new()).await {
        log::warn!("handshake send: {}", e);
        return;
    }

    loop {
        match jsonl::recv_frame(&mut stream).await {
            Ok(jsonl::Frame::Command(cmd)) => {
                let shutdown = matches!(cmd, jsonl::Command::Exit | jsonl::Command::SoftReboot);
                let resp = sysd.handle(&cmd).await;
                if let Err(e) = jsonl::send_frame(&mut stream, &jsonl::Frame::Response(resp)).await
                {
                    log::warn!("send response: {}", e);
                    return;
                }
                if shutdown {
                    let _ = exit_tx.send(()).await;
                    return;
                }
            }
            Ok(jsonl::Frame::Response(_)) => {
                // 协议上 init 不应收到 Response; 忽略.
            }
            Err(jsonl::JsonlError::Eof) => {
                // 客户端 正常关闭连接.
                return;
            }
            Err(e) => {
                log::warn!("recv: {}", e);
                return;
            }
        }
    }
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
