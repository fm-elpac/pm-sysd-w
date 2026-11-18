//! --reload 进程加载 (生产实现).

use std::{env, time::Duration};

use log::warn;
use tokio::{
    io::AsyncReadExt,
    time::{sleep, timeout},
};

use crate::{
    conf::Conf,
    pa::{Process, SpawnOptions},
    sysd::SysdError,
};

/// 软超时: 加载进程运行超时, 仍未返回时, 输出警告, 并继续等待
const LOAD_SOFT_TIMEOUT: Duration = Duration::from_secs(1);

/// 硬超时: 加载进程运行超时后, 终止, 并返回错误
const LOAD_HARD_TIMEOUT: Duration = Duration::from_secs(2);

/// 运行 `pm-sysd-init --reload` 进程, 读取其 stdout 中的 JSON 配置
pub async fn run_reload_subprocess(conf_path: &str) -> Result<Conf, SysdError> {
    let exe = env::current_exe()
        .map_err(|e| SysdError::Failed(format!("current exe: {}", e)))?
        .to_string_lossy()
        .into_owned();
    let argv = vec![exe, "--reload".to_string()];
    let env = vec![("PM_SYSD_CONF".to_string(), conf_path.to_string())];
    let mut proc = Process::spawn(
        &argv,
        None,
        &env,
        SpawnOptions {
            create_no_window: true,
        },
    )
    .map_err(|e| SysdError::Failed(e.to_string()))?;

    let mut stdout = proc.take_stdout().expect("reload stdout piped");
    let mut stderr = proc.take_stderr().expect("reload stderr piped");

    let warn_task = tokio::spawn(async move {
        sleep(LOAD_SOFT_TIMEOUT).await;
        warn!(
            "config load exceeded {} ms, still waiting",
            LOAD_SOFT_TIMEOUT.as_millis()
        );
    });

    let read_fut = async {
        let (out, err, status) =
            tokio::try_join!(read_stdout(&mut stdout), read_stderr(&mut stderr), async {
                proc.wait()
                    .await
                    .map_err(|e| SysdError::Failed(e.to_string()))
            },)?;
        Ok::<_, SysdError>((out, err, status))
    };

    match timeout(LOAD_HARD_TIMEOUT, read_fut).await {
        Ok(Ok((out, err, status))) => {
            warn_task.abort();
            if !status.success() {
                return Err(SysdError::Failed(format!(
                    "reload subprocess exited {}: {}",
                    status,
                    err.trim()
                )));
            }
            serde_json::from_str(&out)
                .map_err(|e| SysdError::Failed(format!("reload subprocess output: {}", e)))
        }
        Ok(Err(e)) => {
            warn_task.abort();
            Err(e)
        }
        Err(_) => {
            warn_task.abort();
            if let Err(e) = proc.kill().await {
                return Err(SysdError::Failed(format!(
                    "config load timed out after {} ms (kill reload subprocess: {})",
                    LOAD_HARD_TIMEOUT.as_millis(),
                    e
                )));
            }
            Err(SysdError::Failed(format!(
                "config load timed out after {} ms",
                LOAD_HARD_TIMEOUT.as_millis()
            )))
        }
    }
}

/// 读取进程 stdout (UTF-8 文本)
async fn read_stdout(r: &mut tokio::process::ChildStdout) -> Result<String, SysdError> {
    let mut s = String::new();
    r.read_to_string(&mut s)
        .await
        .map_err(|e| SysdError::Failed(e.to_string()))?;
    Ok(s)
}

/// 读取进程 stderr (尽力按 UTF-8 解码)
async fn read_stderr(r: &mut tokio::process::ChildStderr) -> Result<String, SysdError> {
    let mut buf = Vec::new();
    r.read_to_end(&mut buf)
        .await
        .map_err(|e| SysdError::Failed(e.to_string()))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}
