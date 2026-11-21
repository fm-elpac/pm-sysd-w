//! --reload 进程加载 (生产实现).

use std::{env, future::Future, pin::Pin, time::Duration};

use log::warn;
use tokio::{
    io::AsyncReadExt,
    time::{sleep, timeout},
};

use crate::{
    conf::{Conf, limit::SysdLimit},
    db::DbClient,
    pa::{NativeProc, SpawnOptions},
    sysd::{
        SysdError,
        reload::{ReloadR, Reloader, assemble},
    },
};

#[cfg(test)]
mod test;

/// 通过 `pm-sysd-init --reload` 子进程加载配置的 Reloader.
///
/// 启动命令/参数由 `cmd` 生成, 测试时可传入别的命令.
pub struct SelfReloader {
    /// 生成启动命令/参数
    cmd: fn() -> Vec<String>,
    /// 限制配置 (供 assemble 打开数据库使用)
    limit: SysdLimit,
    /// 加载进程软超时
    soft_timeout: Duration,
    /// 加载进程硬超时
    hard_timeout: Duration,
}

impl SelfReloader {
    /// 创建 使用自定义命令 的 SelfReloader
    pub fn new(cmd: fn() -> Vec<String>, limit: &SysdLimit) -> Self {
        Self {
            cmd,
            limit: limit.clone(),
            soft_timeout: limit.reload_soft_timeout,
            hard_timeout: limit.reload_hard_timeout,
        }
    }
}

impl Reloader for SelfReloader {
    fn reload<'a>(
        &'a self,
        db: Option<DbClient>,
        conf_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(ReloadR, DbClient), SysdError>> + Send + 'a>> {
        let cmd = self.cmd;
        let soft_timeout = self.soft_timeout;
        let hard_timeout = self.hard_timeout;
        let limit = self.limit.clone();
        Box::pin(async move {
            let conf = run_reload_subprocess(conf_path, cmd, soft_timeout, hard_timeout).await?;
            assemble(db, conf, &limit).await
        })
    }
}

/// 默认的 --reload 启动命令: 启动当前可执行文件自身.
pub fn self_cmd() -> Vec<String> {
    // 当前进程必然有可执行文件路径, 取不到视为 BUG.
    let exe = env::current_exe()
        .expect("current exe must be available")
        .to_string_lossy()
        .into_owned();
    vec![exe, "--reload".to_string()]
}

/// 运行加载子进程, 读取其 stdout 中的 JSON 配置
async fn run_reload_subprocess(
    conf_path: &str,
    cmd: fn() -> Vec<String>,
    soft_timeout: Duration,
    hard_timeout: Duration,
) -> Result<Conf, SysdError> {
    let argv = cmd();
    let env = vec![("PM_SYSD_CONF".to_string(), conf_path.to_string())];
    let mut proc = NativeProc::spawn(
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
        sleep(soft_timeout).await;
        warn!(
            "config load exceeded {} ms, still waiting",
            soft_timeout.as_millis()
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

    match timeout(hard_timeout, read_fut).await {
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
                    hard_timeout.as_millis(),
                    e
                )));
            }
            Err(SysdError::Failed(format!(
                "config load timed out after {} ms",
                hard_timeout.as_millis()
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
