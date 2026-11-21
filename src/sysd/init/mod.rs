//! init 主流程 (pm-sysd-init 的常驻逻辑).
//!
//! 承接 cli::init 解析后的实例与配置路径: 创建 Server 并绑定 IPC (实例唯一性
//! 检测), 异步执行初始化, 然后进入 Server 主循环. 退出 与 soft-reboot 由 SysD
//! 决定, 通过 [`ServerSender`] 通知 Server, 再从 [`Server::loop_accept`] 返回.

use std::time::Duration;

use log::{error, trace, warn};
use tokio::{
    sync::{mpsc, oneshot},
    time::timeout,
};

use crate::{
    conf::limit::SysdLimit,
    db::DbClient,
    jsonl::SdResWhoamiInit,
    logd::Logd,
    pa::{self, Instance},
    sysu::reached_target,
};

pub use super::server::Shutdown;

use super::{
    Server, ServerSender, SysD, SysDM, SysdError, TARGET_DEFAULT,
    reload::{
        Reloader,
        proc::{SelfReloader, self_cmd},
    },
};

#[cfg(test)]
mod test;

/// 启动任务返回给主循环的句柄.
struct StartupR {
    /// 数据库句柄 (退出前 vacuum).
    db: DbClient,
    /// 日志句柄 (退出前 flush).
    logd: Logd,
}

/// init 主流程.
///
/// `print_version` 由 bin 传入 (pm-bin 生成的版本输出函数),
/// 在完成实例唯一性检测后调用, 向控制台输出版本信息.
pub async fn run(
    instance: Instance,
    conf_path: String,
    print_version: fn(),
    limit: SysdLimit,
) -> Result<Shutdown, SysdError> {
    run_with_reloader(
        instance,
        conf_path,
        print_version,
        Box::new(SelfReloader::new(self_cmd, &limit)),
        limit,
    )
    .await
}

/// 供测试注入自定义 [`Reloader`] 的版本 (生产路径由 [`run`] 使用 SelfReloader).
pub async fn run_with_reloader(
    instance: Instance,
    conf_path: String,
    print_version: fn(),
    reloader: Box<dyn Reloader>,
    limit: SysdLimit,
) -> Result<Shutdown, SysdError> {
    trace!("start");

    // (1) 创建 Server 并绑定 IPC (同时完成实例唯一性检测).
    let (mut server, sender) = Server::new(instance.clone(), limit.clone());
    server.bind().await?;
    // 实例唯一性检测已完成, 继续启动前输出版本信息.
    print_version();

    // 启动任务不再阻塞主循环: 主循环立即开始接受连接.
    // 启动任务完成后把 db/logd 句柄回报给主流程 (用于退出清理).
    let (startup_tx, mut startup_rx) = mpsc::channel::<Result<StartupR, SysdError>>(1);
    tokio::spawn(startup(
        conf_path.clone(),
        reloader,
        sender,
        startup_tx,
        instance,
        limit.clone(),
    ));

    // (2) 进入 Server 主循环; 退出/错误从这里返回.
    let shutdown = server.loop_accept().await?;

    // (3) 退出清理: 等待启动结果 (db/logd) 后 flush 与 vacuum.
    match wait_startup(&mut startup_rx, limit.startup_wait_timeout).await {
        Some(r) => {
            if let Err(e) = r.logd.write_with("info", None, "pm-sysd-init exiting") {
                error!("write exit log: {}", e);
            }
            if let Err(e) = r.logd.flush().await {
                error!("flush logs: {}", e);
            }
            // 退出前 (soft-reboot 重启自身前) 对数据库执行 vacuum.
            if let Err(e) = r.db.vacuum().await {
                error!("vacuum database: {}", e);
            }
        }
        None => error!("startup result unavailable, skipping exit flush/vacuum"),
    }
    trace!("exit {:?}", shutdown);
    Ok(shutdown)
}

/// 启动任务: 加载配置, 创建 SysD, 等待 default.target 启动完成.
async fn startup(
    conf_path: String,
    reloader: Box<dyn Reloader>,
    server: ServerSender,
    result_tx: mpsc::Sender<Result<StartupR, SysdError>>,
    instance: Instance,
    limit: SysdLimit,
) {
    let result = startup_inner(&conf_path, reloader, server, instance, &limit).await;
    if result_tx.send(result).await.is_err() {
        error!("send startup result failed");
    }
}

/// 启动任务内部逻辑.
async fn startup_inner(
    conf_path: &str,
    reloader: Box<dyn Reloader>,
    server: ServerSender,
    instance: Instance,
    limit: &SysdLimit,
) -> Result<StartupR, SysdError> {
    // 加载配置与 mask/enable 状态 (首次调用, 数据库由加载线程打开并返回).
    let (loaded, db) = match reloader.reload(None, conf_path).await {
        Ok(x) => x,
        Err(e) => {
            // 启动失败: 让 Server 主循环返回该错误.
            if let Err(e2) = server.exit(Err(e.clone())).await {
                error!("send startup error to server failed: {}", e2);
            }
            return Err(e);
        }
    };
    let vacuum_db = db.clone();

    // 配置加载完成, 更新 Server 的限制配置.
    if let Err(e) = server.set_limit(limit.clone()).await {
        error!("send limit to server failed: {}", e);
    }

    let logd = Logd::new(db.clone(), limit);
    for w in &loaded.conf.warnings {
        warn!("{}", w);
        logd.write_with("warn", None, w).expect("write warning log");
    }

    let whoami = SdResWhoamiInit {
        instance_type: instance.type_name().to_string(),
        instance_name: instance.display_name(),
        pid: std::process::id(),
        service_name: pa::self_service_name().map(str::to_string),
    };

    let sysd = SysD::spawn(
        db,
        logd.clone(),
        conf_path,
        loaded,
        server.clone(),
        reloader,
        whoami,
        limit,
    );

    // 从此处开始 SysD / UnitD / ServiceD 均已创建, 但 default.target 尚未完成.
    if let Err(e) = server.set_sysd(sysd.clone()).await {
        error!("send sysd handle to server failed: {}", e);
    }

    let (tx, rx) = oneshot::channel();
    if sysd.send(SysDM::StartDefaultTarget(tx)).await.is_err() {
        error!("send start-default-target failed (sysd stopped)");
    } else {
        match rx.await {
            Ok(Ok(())) => {
                // 全链路等待完成后, 才输出到达 default.target.
                println!("{}", reached_target(TARGET_DEFAULT));
            }
            Ok(Err(e)) => error!("start default target: {}", e),
            Err(_) => error!("start default target: sysd stopped before reply"),
        }
    }
    logd.write_with("info", None, "pm-sysd-init started")
        .expect("write startup log");

    Ok(StartupR {
        db: vacuum_db,
        logd,
    })
}

/// 等待启动结果 (退出清理用). 超时或启动任务异常时返回 None.
async fn wait_startup(
    rx: &mut mpsc::Receiver<Result<StartupR, SysdError>>,
    startup_wait_timeout: Duration,
) -> Option<StartupR> {
    match timeout(startup_wait_timeout, rx.recv()).await {
        Ok(Some(Ok(r))) => Some(r),
        Ok(Some(Err(e))) => {
            error!("startup failed during exit: {}", e);
            None
        }
        Ok(None) => {
            error!("startup task stopped without result");
            None
        }
        Err(_) => {
            error!("timed out waiting for startup result during exit");
            None
        }
    }
}
