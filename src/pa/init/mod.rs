//! 启动 / 退出 相关封装.

use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
use log::warn;
#[cfg(target_os = "linux")]
use tokio::signal::unix::{SignalKind, signal};

#[cfg(target_os = "windows")]
use tokio::signal::ctrl_c;

#[cfg(target_os = "linux")]
use crate::p_u as p;

#[cfg(target_os = "windows")]
use crate::p_w as p;

use super::PaError;

/// 重启自身 (soft-reboot): 使用相同命令行参数重新运行当前程序.
pub fn soft_reboot(args: &[String]) -> Result<(), PaError> {
    p::soft_reboot(args).map_err(PaError::from)
}

/// 当前进程作为 Windows 服务运行时的服务名 (非服务模式为 `None`).
///
/// 非 Windows 平台恒为 `None`.
pub fn self_service_name() -> Option<&'static str> {
    #[cfg(target_os = "windows")]
    {
        if crate::p_w::service::is_service() {
            Some(crate::p_w::service::SERVICE_NAME)
        } else {
            None
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// 启动外部优雅退出信号监听任务.
///
/// Linux: SIGTERM.
/// Windows: Ctrl-C (命令行) 或 (服务模式) 服务停止请求.
///
/// 收到信号后向 `tx` 发送一次 `()`.
pub fn spawn_signal_watch(tx: mpsc::Sender<()>) {
    #[cfg(unix)]
    {
        tokio::spawn(async move {
            match signal(SignalKind::terminate()) {
                Ok(mut sig) => {
                    sig.recv().await;
                    tx.send(()).await.expect("signal watch receiver dropped");
                }
                Err(e) => {
                    warn!("install SIGTERM handler: {}", e);
                }
            }
        });
    }

    #[cfg(windows)]
    {
        let service = p::service::is_service();
        let stop = p::service::stop_event();

        tokio::spawn(async move {
            if service {
                // Windows 服务模式: 等待 SCM 停止请求 (无控制台, 不监听 Ctrl+C).
                stop.wait().await;
            } else if ctrl_c().await.is_err() {
                // Ctrl+C 监听失败 (例如无控制台): 保持等待, 不发送退出信号.
                return;
            }
            tx.send(()).await.expect("signal watch receiver dropped");
        });
    }
}
