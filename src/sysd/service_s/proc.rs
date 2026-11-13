//! ServiceS 的进程启动 / 退出监视 / Exec* 执行逻辑.

use std::path::Path;

use tokio::sync::{mpsc, oneshot};

use crate::{
    conf::Unit,
    logd::Logd,
    pa::{PaError, Process, SpawnOptions},
};

use super::super::SysdError;
use super::Ev;
use super::actor::ExitInfo;

/// ServiceS 与子进程之间的控制消息
pub enum WatcherCmd {
    /// 终止子进程 (携带退出信息回执)
    Kill(oneshot::Sender<ExitInfo>),
}

/// 启动一个 subprocess, 并接管其 stdout/stderr 写入 unit 日志.
///
/// 返回 [`Process`] (其 stdout/stderr 已交由后台任务按行读取).
fn spawn_with_log(
    argv: &[String],
    cwd: Option<&Path>,
    env: &[(String, String)],
    opts: SpawnOptions,
    name: &str,
    logd: &Logd,
) -> Result<Process, SysdError> {
    let mut child =
        Process::spawn(argv, cwd, env, opts).map_err(|e| SysdError::Failed(e.to_string()))?;

    if let Some(err) = child.take_stderr() {
        logd.spawn_log_reader(name, "stderr", err);
    }
    if let Some(out) = child.take_stdout() {
        logd.spawn_log_reader(name, "stdout", out);
    }
    Ok(child)
}

/// 启动一个 service 的主进程 (ExecStart)
pub fn spawn_child(unit: &Unit, logd: &Logd) -> Result<Process, SysdError> {
    let argv = unit.service.exec_start.as_slice();
    if argv.is_empty() {
        return Err(SysdError::Failed(format!(
            "unit `{}` ExecStart is empty",
            unit.name
        )));
    }
    let cwd = unit.service.working_directory.as_deref().map(Path::new);
    let child = spawn_with_log(
        argv,
        cwd,
        &unit.service.environment,
        SpawnOptions {
            create_no_window: unit.win.create_no_window,
        },
        &unit.name,
        logd,
    )?;
    logd.write_with(
        "info",
        Some(&unit.name),
        &format!("started (pid {})", child.id().unwrap_or(0)),
    )
    .expect("write started log");
    Ok(child)
}

/// 运行一个 Exec* 命令 (ExecReload / ExecStop): 启动 subprocess 并等待其退出.
pub async fn run_exec(
    cmd: &[String],
    unit: &Unit,
    name: &str,
    logd: &Logd,
) -> Result<(), SysdError> {
    if cmd.is_empty() {
        return Err(SysdError::Failed(format!(
            "unit `{}` exec command is empty",
            name
        )));
    }
    logd.write_with(
        "info",
        Some(name),
        &format!(
            "exec started {}",
            serde_json::to_string(cmd).unwrap_or_default()
        ),
    )
    .expect("write exec started log");
    let cwd = unit.service.working_directory.as_deref().map(Path::new);
    let mut child = spawn_with_log(
        cmd,
        cwd,
        &unit.service.environment,
        SpawnOptions {
            create_no_window: unit.win.create_no_window,
        },
        name,
        logd,
    )?;
    let status = child
        .wait()
        .await
        .map_err(|e| SysdError::Failed(format!("exec `{}`: {}", name, e)))?;
    if status.success() {
        logd.write_with("info", Some(name), "exec exited (status 0)")
            .expect("write exec exited log");
        Ok(())
    } else {
        logd.write_with("error", Some(name), &format!("exec failed ({})", status))
            .expect("write exec failed log");
        Err(SysdError::Failed(format!(
            "exec `{}` failed: {}",
            name, status
        )))
    }
}

/// 把 `Process::wait` 的结果转换为 [`ExitInfo`]
fn exit_info(r: Result<std::process::ExitStatus, PaError>) -> ExitInfo {
    match r {
        Ok(status) => ExitInfo {
            success: status.success(),
            code: status.code(),
        },
        Err(_) => ExitInfo {
            success: false,
            code: None,
        },
    }
}

/// 启动子进程退出监视 (事件驱动, 不轮询).
///
/// 持有子进程, 等待其自然退出; 收到终止命令时终止并等待退出.
pub fn spawn_watcher(
    mut child: Process,
    natural_tx: mpsc::Sender<ExitInfo>,
    mut kill_rx: mpsc::Receiver<WatcherCmd>,
) {
    tokio::spawn(async move {
        let mut natural_status: Option<ExitInfo> = None;
        let mut pending_kill: Option<oneshot::Sender<ExitInfo>> = None;

        loop {
            {
                let exit = child.wait();
                tokio::pin!(exit);
                tokio::select! {
                    s = exit => {
                        natural_status = Some(exit_info(s));
                    }
                    cmd = kill_rx.recv() => {
                        match cmd {
                            Some(WatcherCmd::Kill(reply)) => pending_kill = Some(reply),
                            None => return,
                        }
                    }
                }
            }

            if let Some(info) = natural_status {
                natural_tx
                    .send(info)
                    .await
                    .expect("natural exit channel closed");
                if let Some(reply) = pending_kill.take() {
                    reply.send(info).expect("kill reply channel closed");
                }
                while let Some(cmd) = kill_rx.recv().await {
                    let WatcherCmd::Kill(reply) = cmd;
                    reply.send(info).expect("kill reply channel closed");
                }
                return;
            }

            if let Some(reply) = pending_kill.take() {
                child.kill().await.expect("kill child");
                let s = child.wait().await;
                reply.send(exit_info(s)).expect("kill reply channel closed");
                return;
            }
        }
    });
}

/// 把自然退出事件转发到 ServiceS 的内部事件通道
pub fn spawn_forwarder(mut natural_rx: mpsc::Receiver<ExitInfo>, ev_tx: mpsc::Sender<Ev>) {
    tokio::spawn(async move {
        while let Some(info) = natural_rx.recv().await {
            if ev_tx.send(Ev::ChildExited(info)).await.is_err() {
                break;
            }
        }
    });
}
