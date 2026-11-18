//! 日志核心逻辑 (类似 journald, 但简化实现).
//!
//! 日志数据存入 pm-sysd-init 的 sqlite 数据库.
//!
//! 本模块负责日志的写入 (含运行中进程 stdout/stderr 的采集),
//! 查询与自动清理:
//!
//! - 写入采用专用 (同步) 线程 + MPSC: 发送方无阻塞 (`try_send`), 接收方
//!   使用较大缓冲区 (1000 条) 缓存来不及写入的日志, 避免日志写入阻塞主逻辑.
//!   日志写入线程位于 `db` 模块 (`db::DbClient`), 本模块只负责拆分/投递.
//!
//! - 子进程 stdout/stderr 由后台任务按行 (超长分段) 读取后写入对应 unit 日志.
//!
//! - 日志查询命令 (`SdReqLogRO`) 由后台 task 处理, 返回分页响应.
//!
//! 防御性处理 (详见 `doc/dev/README.md`):
//!
//! - 单条日志最大 [`MAX_LOG_LEN`] 字节, 超长由发送方拆分为多条.
//! - 发送失败 (缓冲区满 / 写入线程停止) 时丢弃并限速警告.
//! - 日志自动清理: 每 [`LOG_CLEAN_INTERVAL`] 运行一次, 只保留最近
//!   [`LOG_RETENTION`] 的日志.

use std::time::{Duration, Instant};

use chrono::{SecondsFormat, Utc};
use log::{info, warn};
use tokio::{
    sync::{mpsc, oneshot},
    time::sleep,
};

use crate::{
    db::{DbClient, DbError},
    jsonl::{SdReqLogRO, SdRes},
};

mod error;
mod proc;
mod query;
mod warn;

#[cfg(test)]
mod test;

pub use error::LogdError;
pub use query::LogPage;
pub use warn::{WARN_RATE_LIMIT, warn_rate_limited};

/// 单条日志最大长度 (字节), 超出由发送方拆分为多条
pub const MAX_LOG_LEN: usize = 500 * 1024;

/// 日志自动清理间隔 (固定: 每 1 小时一次)
pub const LOG_CLEAN_INTERVAL: Duration = Duration::from_secs(3600);

/// 日志保留时长 (固定: 最近 72 小时)
pub const LOG_RETENTION: Duration = Duration::from_secs(72 * 3600);

/// 单次日志分页查询允许的最大条数 (硬限制).
///
/// 外部请求的一页条数超过此值时, 自动按此值截断.
pub const LOG_PAGE_MAX: u32 = 10_000;

/// 日志记录器.
///
/// clone: 共享同一个数据库客户端 (数据库读写线程与日志写入线程) 与日志查询
/// task 的消息通道.
#[derive(Clone)]
pub struct Logd {
    /// 数据库客户端 (异步读写与日志写入)
    db: DbClient,
    /// 日志查询 task 的消息通道 (只负责转发查询请求)
    tx: mpsc::Sender<(SdReqLogRO, oneshot::Sender<SdRes>)>,
}

impl Logd {
    /// 创建日志记录器, 并启动日志清理 task 与日志查询 task.
    ///
    /// 数据库读写线程与日志写入线程已由 [`DbClient::open`] 启动.
    pub fn new(db: DbClient) -> Self {
        // 日志自动清理 task
        let clean_db = db.clone();
        tokio::spawn(async move {
            loop {
                sleep(LOG_CLEAN_INTERVAL).await;

                let before = (Utc::now()
                    - chrono::Duration::seconds(LOG_RETENTION.as_secs() as i64))
                .to_rfc3339_opts(SecondsFormat::Millis, true);

                match clean_db.log_delete_before(&before).await {
                    Ok(n) => {
                        if n > 0 {
                            info!("log cleanup: deleted {} old logs", n);
                        }
                    }
                    Err(e) => warn!("log cleanup: {}", e),
                }
            }
        });

        // 日志查询 task
        let (tx, rx) = mpsc::channel(64);
        let query_db = db.clone();
        tokio::spawn(async move { query::run(rx, query_db).await });

        Self { db, tx }
    }

    /// 写入一条日志 (级别 `info`, 无关联 unit)
    pub fn write(&mut self, msg: &str) -> Result<(), LogdError> {
        self.write_with("info", None, msg)
    }

    /// 写入一条日志 (指定级别 / 关联 unit).
    ///
    /// 无阻塞: 仅把日志投递到写入线程的缓冲区, 不等待数据库写入完成.
    /// 超长日志会先拆分为多条. 缓冲区满或写入线程停止时丢弃并限速警告.
    pub fn write_with(&self, level: &str, unit: Option<&str>, msg: &str) -> Result<(), LogdError> {
        let ts = now_iso();
        let level = level.to_string();
        let unit = unit.map(|s| s.to_string());

        for chunk in split_msg(msg, MAX_LOG_LEN) {
            match self.db.log_append(&ts, &level, unit.as_deref(), &chunk) {
                Ok(()) => {}
                Err(DbError::Busy(_)) => {
                    warn_rate_limited("log-buffer-full", "log buffer full; dropping log");
                }
                Err(_) => {
                    warn_rate_limited("log-writer-stopped", "log writer stopped; dropping log");
                }
            }
        }
        Ok(())
    }

    /// 等待之前投递的日志全部写入数据库 (用于优雅退出与测试)
    pub async fn flush(&self) -> Result<(), LogdError> {
        self.db
            .log_flush()
            .await
            .map_err(|e| LogdError::Db(e.to_string()))
    }
}

/// 把超长日志按 UTF-8 字符边界拆分为若干段
fn split_msg(msg: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = msg;
    while rest.len() > max {
        let mut end = max;
        while end > 0 && !rest.is_char_boundary(end) {
            end -= 1;
        }
        out.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    out.push(rest.to_string());
    out
}

/// 当前时间 (UTC, ISO 8601, 毫秒精度)
pub fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// 当前时间 (UTC, Unix 毫秒时间戳).
///
/// 用于记录 unit 运行状态的变化时间 (`status` 的 `since` 部分).
pub fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

/// 把单调时钟 [`Instant`] 近似转换为 ISO 8601 时间戳 (UTC, 毫秒精度).
///
/// `Instant` 无法直接对应墙上时间, 这里用当前墙上时间减去该 `Instant` 已流逝的
/// 时长来反推一个近似值, 仅用于调试 / 日志序列化.
pub fn instant_iso(instant: &Instant) -> String {
    let elapsed = chrono::Duration::from_std(instant.elapsed())
        .expect("instant elapsed is within chrono duration range");
    (Utc::now() - elapsed).to_rfc3339_opts(SecondsFormat::Millis, true)
}
