//! 日志 核心逻辑 (类似 journald, 但简化实现).
//!
//! 日志数据 存入 pm-sysd-init 的 sqlite 数据库.
//!
//! 本模块 负责 日志的 写入 (含 运行中进程 stdout/stderr 的 采集), 查询 与
//! 自动清理:
//!
//! - 写入采用 专用 (同步) 线程 + MPSC: 发送方 无阻塞 (`try_send`), 接收方
//!   使用较大缓冲区 (1000 条) 缓存 来不及写入的日志, 避免 日志写入 阻塞 主逻辑.
//!   日志写入线程 位于 `db` 模块 (`db::DbClient`), 本模块只负责 拆分/投递.
//! - 子进程 stdout/stderr 由 后台任务 按行读取 后 写入 对应 unit 日志.
//! - 日志查询命令 (`SdReqLogRO`) 由 后台 task 处理, 返回 分页 响应.
//!
//! 防御性处理 (详见 `doc/dev/README.md`):
//!
//! - 单条日志 最大 [`MAX_LOG_LEN`] 字节, 超长 由发送方拆分为多条.
//! - 发送失败 (缓冲区满 / 写入线程停止) 时 丢弃并 限速警告.
//! - 日志 自动清理: 每 [`LOG_CLEAN_INTERVAL`] 运行一次, 只保留最近
//!   [`LOG_RETENTION`] 的日志.

use std::{
    str,
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use chrono::{SecondsFormat, Utc};
use log::{info, warn};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    sync::{mpsc, oneshot},
};

use crate::{
    db::{DbClient, DbError, LogEntry, MAX_LOG_LEN},
    jsonl::{SdReqLogRO, SdRes, SdResLogEntry as JsonLogEntry},
};

mod error;

#[cfg(test)]
mod test;

pub use error::LogdError;

/// 丢弃日志 警告 的 最小间隔 (速率限制).
const WARN_RATE_LIMIT: Duration = Duration::from_secs(1);

/// 日志 自动清理 间隔 (固定: 每 1 小时 一次).
const LOG_CLEAN_INTERVAL: Duration = Duration::from_secs(3600);

/// 日志 保留时长 (固定: 最近 72 小时).
const LOG_RETENTION: Duration = Duration::from_secs(72 * 3600);

/// 日志分页结果.
#[derive(Debug, Clone)]
pub struct LogPage {
    /// 匹配条件的日志总量.
    pub total: u64,
    /// 下一页游标 (无更多日志时为空).
    pub cursor: Option<i64>,
    /// 本页日志列表 (按 id 升序).
    pub entries: Vec<LogEntry>,
}

/// 日志记录器.
///
/// clone: 共享同一个数据库客户端 (数据库读写线程 与 日志写入线程) 与 日志查询
/// task 的 消息通道.
#[derive(Clone)]
pub struct Logd {
    /// 数据库客户端 (异步读写 与 日志写入).
    db: DbClient,
    /// 日志查询 task 的消息通道 (只负责转发查询请求).
    tx: mpsc::Sender<(SdReqLogRO, oneshot::Sender<SdRes>)>,
}

impl Logd {
    /// 创建日志记录器, 并启动 日志清理线程 与 日志查询 task.
    ///
    /// 数据库读写线程 与 日志写入线程 已由 [`DbClient::open`] 启动.
    pub fn new(db: DbClient) -> Self {
        // 日志 自动清理线程.
        let clean_db = db.clone();
        thread::Builder::new()
            .name("pm-sysd-w-log-clean".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(LOG_CLEAN_INTERVAL);
                    let before = (Utc::now()
                        - chrono::Duration::seconds(LOG_RETENTION.as_secs() as i64))
                    .to_rfc3339_opts(SecondsFormat::Millis, true);
                    match clean_db.log_delete_before_blocking(&before) {
                        Ok(n) => {
                            if n > 0 {
                                info!("log cleanup: deleted {} old logs", n);
                            }
                        }
                        Err(e) => warn!("log cleanup: {}", e),
                    }
                }
            })
            .expect("spawn log clean thread");

        // 日志查询 task.
        let (tx, rx) = mpsc::channel(64);
        let query_db = db.clone();
        tokio::spawn(async move { run(rx, query_db).await });

        Self { db, tx }
    }

    /// 发送 日志查询 请求 (等待 投递完成, 不等待 处理/回执).
    pub async fn req_log_ro(
        &self,
        cmd: SdReqLogRO,
        reply: oneshot::Sender<SdRes>,
    ) -> Result<(), mpsc::error::SendError<(SdReqLogRO, oneshot::Sender<SdRes>)>> {
        self.tx.send((cmd, reply)).await
    }

    /// 写入一条日志 (级别 `info`, 无关联 unit).
    pub fn write(&mut self, msg: &str) -> Result<(), LogdError> {
        self.write_with("info", None, msg)
    }

    /// 写入一条日志 (指定级别 / 关联 unit).
    ///
    /// 无阻塞: 仅把日志投递到 写入线程 的缓冲区, 不等待数据库写入完成.
    /// 超长日志 会先拆分为 多条. 缓冲区满或写入线程停止时 丢弃并限速警告.
    pub fn write_with(&self, level: &str, unit: Option<&str>, msg: &str) -> Result<(), LogdError> {
        let ts = now_iso();
        let level = level.to_string();
        let unit = unit.map(|s| s.to_string());

        for chunk in split_msg(msg, MAX_LOG_LEN) {
            match self.db.log_append(&ts, &level, unit.as_deref(), &chunk) {
                Ok(()) => {}
                Err(DbError::Busy(_)) => {
                    warn_rate_limited("log buffer full; dropping log");
                }
                Err(_) => {
                    warn_rate_limited("log writer stopped; dropping log");
                }
            }
        }
        Ok(())
    }

    /// 启动 后台任务: 按行读取 子进程的 stdout/stderr, 写入对应 unit 日志.
    ///
    /// `level` 分别为 `stdout` / `stderr`.
    pub fn spawn_log_reader<R>(&self, name: &str, level: &str, reader: R)
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let logd = self.clone();
        let name = name.to_string();
        let level = level.to_string();
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut buf: Vec<u8> = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf).await {
                    // 读到 EOF: 管道关闭 (子进程 退出), 结束本任务.
                    Ok(0) => break,
                    Ok(_) => {
                        // 去掉 行尾 换行符 (Windows `\r\n` / Unix `\n`).
                        while buf.last() == Some(&b'\n') || buf.last() == Some(&b'\r') {
                            buf.pop();
                        }
                        match str::from_utf8(&buf) {
                            // 超长行 由 logd 自动拆分为多条日志.
                            Ok(s) => {
                                logd.write_with(&level, Some(&name), s)
                                    .expect("write subprocess output log");
                            }
                            // 非 UTF-8 输出: 换成一条 error 日志, 写入对应 unit.
                            Err(_) => {
                                logd.write_with(
                                    "error",
                                    Some(&name),
                                    &format!(
                                        "subprocess {} output is not utf-8; replaced with this error",
                                        level
                                    ),
                                )
                                .expect("write subprocess non-utf-8 log");
                            }
                        }
                    }
                    Err(e) => {
                        warn!("read `{}` {}: {}", name, level, e);
                        break;
                    }
                }
            }
        });
    }

    /// 等待 之前投递的 日志 全部写入数据库 (用于 优雅退出 与 测试).
    pub fn flush(&self) -> Result<(), LogdError> {
        self.db
            .log_flush()
            .map_err(|e| LogdError::Db(e.to_string()))
    }

    /// 读取 最近的 若干条日志 (可选 按 unit 过滤).
    pub async fn recent(&self, unit: Option<&str>, limit: u32) -> Result<Vec<String>, LogdError> {
        Ok(self
            .recent_entries(unit, limit)
            .await?
            .into_iter()
            .map(|e| match e.unit {
                Some(u) => format!("{} [{}] {}: {}", e.ts, e.level, u, e.msg),
                None => format!("{} [{}] {}", e.ts, e.level, e.msg),
            })
            .collect())
    }

    /// 读取 最近的 若干条日志 (结构化, 可选 按 unit 过滤).
    pub async fn recent_entries(
        &self,
        unit: Option<&str>,
        limit: u32,
    ) -> Result<Vec<LogEntry>, LogdError> {
        self.db
            .log_recent(unit, limit)
            .await
            .map_err(|e| LogdError::Db(e.to_string()))
    }

    /// 分页读取日志 (可选 按 unit 过滤).
    ///
    /// `cursor` 为 `Some(id)` 时 读取 `id < cursor` 的日志; 为 `None` 时 读取最新.
    /// 返回 总量 / 下一页游标 / 本页日志 (按 id 升序).
    pub async fn page_entries(
        &self,
        unit: Option<&str>,
        limit: u32,
        cursor: Option<i64>,
    ) -> Result<LogPage, LogdError> {
        page_entries_with(&self.db, unit, limit, cursor).await
    }
}

/// 分页读取日志 (供 日志查询 task 与 [`Logd::page_entries`] 使用).
async fn page_entries_with(
    db: &DbClient,
    unit: Option<&str>,
    limit: u32,
    cursor: Option<i64>,
) -> Result<LogPage, LogdError> {
    let total = db
        .log_total(unit)
        .await
        .map_err(|e| LogdError::Db(e.to_string()))?;

    // 多取 1 条, 用于判断 是否还有下一页.
    let fetch = limit.saturating_add(1).max(1);
    let mut rows = db
        .log_page_raw(unit, fetch, cursor)
        .await
        .map_err(|e| LogdError::Db(e.to_string()))?;

    let has_more = rows.len() as u64 > limit as u64;
    if has_more {
        rows.truncate(limit as usize);
    }

    // 当前页 按 id 倒序 取到; 记录 本页最旧 id 作为 下一页游标.
    let next_cursor = if has_more {
        rows.last().map(|e| e.id)
    } else {
        None
    };
    rows.reverse();

    Ok(LogPage {
        total,
        cursor: next_cursor,
        entries: rows,
    })
}

/// 处理 日志查询 请求, 返回 响应.
async fn log_ro_resp(db: &DbClient, cmd: &SdReqLogRO) -> SdRes {
    let r: Result<SdRes, LogdError> = match cmd {
        SdReqLogRO::Query(query) => {
            let page = match page_entries_with(db, query.unit.as_deref(), query.limit, query.cursor)
                .await
            {
                Ok(page) => page,
                Err(e) => return SdRes::Err(e.to_string()),
            };
            Ok(SdRes::Logs {
                total: page.total,
                cursor: page.cursor,
                logs: page
                    .entries
                    .into_iter()
                    .map(|e| JsonLogEntry {
                        id: e.id,
                        ts: e.ts,
                        level: e.level,
                        unit: e.unit,
                        msg: e.msg,
                    })
                    .collect(),
            })
        }
    };
    match r {
        Ok(resp) => resp,
        Err(e) => SdRes::Err(e.to_string()),
    }
}

/// 日志查询 task 主循环.
async fn run(mut rx: mpsc::Receiver<(SdReqLogRO, oneshot::Sender<SdRes>)>, db: DbClient) {
    while let Some((cmd, reply)) = rx.recv().await {
        reply.send(log_ro_resp(&db, &cmd).await).unwrap();
    }
}

/// 把超长日志 按 UTF-8 字符边界 拆分为 若干段.
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

/// 限速警告: 同一条消息 至少间隔 [`WARN_RATE_LIMIT`] 才输出一次.
fn warn_rate_limited(msg: &str) {
    static LAST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    let last = LAST.get_or_init(|| Mutex::new(None));
    let mut guard = last.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();

    let allow = match *guard {
        Some(t) => now.duration_since(t) >= WARN_RATE_LIMIT,
        None => true,
    };
    if allow {
        warn!("{}", msg);
        *guard = Some(now);
    }
}

/// 当前时间 (UTC, ISO 8601, 毫秒精度).
fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
