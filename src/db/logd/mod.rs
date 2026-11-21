//! 日志相关的数据库读写与后台日志写入线程.

#[cfg(test)]
mod test;

use std::{
    sync::{Weak, mpsc},
    time::Duration,
};

use log::warn;
use rusqlite::ToSql;
use tokio::sync::oneshot;

use crate::logd::warn_rate_limited;

use super::{
    DbClient, DbError,
    rw::{DbRequest, recv_resp, send_req},
    util::Db,
};

/// 一条日志记录
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// 自增 id
    pub id: i64,
    /// 时间戳 (UTC, ISO 8601)
    pub ts: String,
    /// 日志级别 (如 `debug`/`info`/`warn`/`error`)
    pub level: String,
    /// 关联 unit 名 (可为空)
    pub unit: Option<String>,
    /// 日志内容
    pub msg: String,
}

/// 日志分页原始查询结果
#[derive(Debug, Clone)]
pub struct LogPageRaw {
    /// 匹配条件的日志总量 (不受分页影响)
    pub total: u64,
    /// 查询到的日志 (按请求方向排序)
    pub entries: Vec<LogEntry>,
    /// 是否因累计日志内容长度达到上限而提前停止 (仍有更多匹配日志)
    pub size_limited: bool,
}

/// 日志查询开始位置 (数据库内部表示).
///
/// 与 jsonl 的 [`SdReqPagePos`](crate::jsonl::SdReqPagePos) 对应, 不引入 jsonl 依赖.
/// 返回条数由查询方向 (`asc`) 与 `limit` 决定, 本枚举不限定范围.
#[derive(Debug, Clone, Copy)]
pub enum LogPos {
    /// 从游标的下一条/上一条开始 (不含游标), 方向由 `asc` 决定
    Cursor(i64),
    /// 从最旧数据开始, 向后跳过 `n` 条
    Start(usize),
    /// 从最新数据开始, 向前跳过 `n` 条
    End(usize),
}

/// 一条待写入的日志
pub(super) struct LogMsg {
    pub ts: String,
    pub level: String,
    pub unit: Option<String>,
    pub msg: String,
}

/// 发给日志批发线程的消息 (批发线程只读取这一个 MPSC 通道)
pub(super) enum LogIn {
    /// 写入一条日志
    Log(LogMsg),
    /// 批发请求 (日志写入线程发来, 请求取走一批日志)
    Batch,
    /// 请求 flush (批发线程确认之前所有日志已写入后回执)
    Flush(oneshot::Sender<()>),
}

/// 日志批发线程发给日志写入线程的一批日志
pub(super) enum WriterCmd {
    /// 一批待写入的日志
    Batch(Vec<LogMsg>),
}

/// 日志相关的数据库操作.
pub(super) struct DbLog {
    db: Db,
}

impl DbLog {
    /// 新建 `DbLog` (内部持有共享的 `Db` 连接).
    pub(super) fn new(db: Db) -> Self {
        Self { db }
    }

    /// 批量追加多条日志 (单个事务)
    fn log_append_batch(&self, logs: &[LogMsg]) -> Result<(), DbError> {
        self.db.with_tx(|tx| {
            let mut stmt = tx
                .prepare("INSERT INTO log (ts, level, unit, msg) VALUES (?1, ?2, ?3, ?4)")
                .map_err(|e| DbError::Sqlite(format!("log_append_batch: {}", e)))?;
            for m in logs {
                let ts = m.ts.as_str();
                let level = m.level.as_str();
                let unit = m.unit.as_deref();
                let msg = m.msg.as_str();
                stmt.execute(rusqlite::params![ts, level, unit, msg])
                    .map_err(|e| DbError::Sqlite(format!("log_append_batch: {}", e)))?;
            }
            Ok(())
        })
    }

    /// 读取日志分页 (按请求方向排序), 并返回匹配条件的日志总量.
    ///
    /// `asc` 为 `true` 时按 id 升序, `false` 时按 id 降序.
    /// `pos` 为开始位置 (游标 / 跳过条数), 详见 [`LogPos`].
    /// `limit` 为最多读取条数, 上层通常会多取 1 条以判断是否还有下一页.
    /// `max_msg_bytes` 为累计日志内容 (msg) 长度上限, 超出则提前停止.
    pub(super) fn log_page(
        &self,
        level: Option<&str>,
        unit: Option<&str>,
        asc: bool,
        pos: LogPos,
        limit: u32,
        max_msg_bytes: usize,
    ) -> Result<LogPageRaw, DbError> {
        self.db.with_tx(|tx| {
            let (cond_sql, params) = log_conds(level, unit);
            let count_sql = if cond_sql.is_empty() {
                "SELECT COUNT(*) FROM log".to_string()
            } else {
                format!("SELECT COUNT(*) FROM log WHERE {}", cond_sql)
            };
            let total: i64 = tx
                .query_row(&count_sql, dyn_params(&params).as_slice(), |row| row.get(0))
                .map_err(|e| DbError::Sqlite(format!("log_page: {}", e)))?;
            let total_u = total as u64;

            let order = if asc { "ASC" } else { "DESC" };

            // 根据开始位置计算: 游标条件 / 结果顺序中的偏移.
            // 返回条数只由 `limit` 决定, 不在此处截断范围.
            let mut cond_sql = cond_sql;
            let mut params = params;
            let mut offset: u64 = 0;

            match pos {
                LogPos::Cursor(id) => {
                    let op = if asc { ">" } else { "<" };
                    let extra = format!("(id {} ?)", op);
                    cond_sql = if cond_sql.is_empty() {
                        extra
                    } else {
                        format!("{} AND {}", cond_sql, extra)
                    };
                    params.push(Box::new(id));
                }
                LogPos::Start(n) => {
                    let n = n as u64;
                    offset = if asc {
                        n
                    } else if n < total_u {
                        total_u - 1 - n
                    } else {
                        // 跳过条数超过已有条数, 结果为空.
                        total_u
                    };
                }
                LogPos::End(n) => {
                    let n = n as u64;
                    offset = if asc {
                        if n < total_u {
                            total_u - 1 - n
                        } else {
                            // 跳过条数超过已有条数时, 升序从最旧数据开始.
                            0
                        }
                    } else {
                        n
                    };
                }
            }

            let effective_limit = limit as u64;

            let where_sql = if cond_sql.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", cond_sql)
            };
            let sql = format!(
                "SELECT id, ts, level, unit, msg FROM log {} ORDER BY id {} LIMIT ? OFFSET ?",
                where_sql, order
            );
            let mut stmt = tx
                .prepare(&sql)
                .map_err(|e| DbError::Sqlite(format!("log_page: {}", e)))?;

            let limit_i64 = effective_limit.min(i64::MAX as u64) as i64;
            let offset_i64 = offset.min(i64::MAX as u64) as i64;
            let mut param_refs = dyn_params(&params);
            param_refs.push(&limit_i64);
            param_refs.push(&offset_i64);

            let rows = stmt
                .query_map(param_refs.as_slice(), log_from_row)
                .map_err(|e| DbError::Sqlite(format!("log_page: {}", e)))?;

            let mut out = Vec::new();
            let mut total_bytes = 0usize;
            let mut size_limited = false;
            for r in rows {
                let e = r.map_err(|e| DbError::Sqlite(format!("log_page: {}", e)))?;
                // 至少返回 1 条, 之后按累计内容长度提前停止.
                if !out.is_empty() && total_bytes.saturating_add(e.msg.len()) > max_msg_bytes {
                    size_limited = true;
                    break;
                }
                total_bytes = total_bytes.saturating_add(e.msg.len());
                out.push(e);
            }
            Ok(LogPageRaw {
                total: total_u,
                entries: out,
                size_limited,
            })
        })
    }

    /// 删除时间戳早于 `ts` 的日志, 返回删除条数 (日志自动清理)
    pub(super) fn log_delete_before(&self, ts: &str) -> Result<u64, DbError> {
        self.db.with_tx(|tx| {
            let n = tx
                .execute("DELETE FROM log WHERE ts < ?1", rusqlite::params![ts])
                .map_err(|e| DbError::Sqlite(format!("log_delete_before: {}", e)))?;
            Ok(n as u64)
        })
    }
}

impl DbClient {
    /// 读取日志分页 (按请求方向排序), 并返回匹配条件的日志总量.
    ///
    /// `asc` 为 `true` 时按 id 升序, `false` 时按 id 降序.
    /// `pos` 为开始位置, 详见 [`LogPos`].
    /// `max_msg_bytes` 为累计日志内容 (msg) 长度上限, 超出则提前停止, 避免一次读取过大.
    /// 若存在匹配日志, 至少返回 1 条.
    pub async fn log_page(
        &self,
        level: Option<&str>,
        unit: Option<&str>,
        asc: bool,
        pos: LogPos,
        limit: u32,
        max_msg_bytes: usize,
    ) -> Result<LogPageRaw, DbError> {
        let (resp, rx) = oneshot::channel();
        send_req(
            &self.req_tx,
            DbRequest::LogPage {
                level: level.map(|s| s.to_string()),
                unit: unit.map(|s| s.to_string()),
                asc,
                pos,
                limit,
                max_msg_bytes,
                resp,
            },
        )?;
        recv_resp(rx).await
    }

    /// 删除时间戳早于 `ts` 的日志, 返回删除条数.
    pub async fn log_delete_before(&self, ts: &str) -> Result<u64, DbError> {
        let (resp, rx) = oneshot::channel();
        send_req(
            &self.req_tx,
            DbRequest::LogDeleteBefore {
                ts: ts.to_string(),
                resp,
            },
        )?;
        recv_resp(rx).await
    }

    /// 追加一条日志 (非阻塞, 仅投递到日志批发线程)
    pub fn log_append(
        &self,
        ts: &str,
        level: &str,
        unit: Option<&str>,
        msg: &str,
    ) -> Result<(), DbError> {
        let cmd = LogIn::Log(LogMsg {
            ts: ts.to_string(),
            level: level.to_string(),
            unit: unit.map(|s| s.to_string()),
            msg: msg.to_string(),
        });
        match self.log_tx.try_send(cmd) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => Err(DbError::Busy("log buffer full".into())),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                Err(DbError::Msg("log writer stopped".into()))
            }
        }
    }

    /// 等待之前投递的日志全部写入数据库 (用于优雅退出与测试)
    pub async fn log_flush(&self) -> Result<(), DbError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let tx = self.log_tx.clone();
        // 用 spawn_blocking 执行阻塞的 send, 避免阻塞 async 线程
        let send_res = tokio::task::spawn_blocking(move || {
            tx.send(LogIn::Flush(ack_tx))
                .map_err(|_| DbError::Msg("log writer stopped".into()))
        })
        .await
        .map_err(|e| DbError::Msg(format!("log flush task failed: {}", e)))?;

        send_res?;
        ack_rx
            .await
            .map_err(|_| DbError::Msg("log writer stopped".into()))
    }
}

/// 日志批发线程轮询退出信号的间隔.
///
/// 仅在无消息时使用, 用于在 `DbClient` 全部释放后退出线程. 有消息到达时立即返回,
/// 不影响日志吞吐.
const SHUTDOWN_POLL: Duration = Duration::from_millis(100);

/// 日志写入线程主循环.
///
/// 循环: 向批发线程请求一批日志, 等待批数据, 然后在单个事务内批量写入数据库.
/// 空闲时阻塞等待批发线程发来数据; 同时定期检查 `DbClient` 是否已全部释放.
pub(super) fn log_writer(
    log: DbLog,
    in_tx: mpsc::SyncSender<LogIn>,
    rx: mpsc::Receiver<WriterCmd>,
    log_line_byte: usize,
    warn_interval_min: Duration,
    life: Weak<()>,
) {
    loop {
        // 请求一批日志. 发送失败表示批发线程已退出, 本线程也退出.
        if in_tx.send(LogIn::Batch).is_err() {
            return;
        }

        // 等待批发线程发来一批日志.
        let batch = loop {
            match rx.recv_timeout(SHUTDOWN_POLL) {
                Ok(WriterCmd::Batch(b)) => break b,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if life.upgrade().is_none() {
                        return;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        };

        // 防御性检查: 正常情况下发送方已拆分, 不应出现超长日志.
        let mut valid = Vec::with_capacity(batch.len());
        for m in batch {
            if m.msg.len() > log_line_byte {
                warn_rate_limited(
                    "log-over-long",
                    "log writer: dropping over-long log",
                    warn_interval_min,
                );
                continue;
            }
            valid.push(m);
        }

        if !valid.is_empty() {
            if let Err(e) = log.log_append_batch(&valid) {
                warn!("log write: {}", e);
            }
        }
    }
}

/// 日志批发线程的内部缓冲区.
struct LogBuffer {
    logs: Vec<LogMsg>,
    /// 当前缓冲区中日志数据总大小 (字节)
    bytes: usize,
}

impl LogBuffer {
    fn new() -> Self {
        Self {
            logs: Vec::new(),
            bytes: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.logs.is_empty()
    }

    fn len(&self) -> usize {
        self.logs.len()
    }

    fn push(&mut self, m: LogMsg, size: usize) {
        self.bytes = self.bytes.saturating_add(size);
        self.logs.push(m);
    }

    /// 取走全部缓冲日志并清空计数.
    fn take(&mut self) -> Vec<LogMsg> {
        self.bytes = 0;
        std::mem::take(&mut self.logs)
    }
}

/// 计算一条日志数据的大小 (ts + level + unit + msg 的字节数).
fn log_msg_size(m: &LogMsg) -> usize {
    m.ts.len()
        .saturating_add(m.level.len())
        .saturating_add(m.unit.as_ref().map_or(0, |u| u.len()))
        .saturating_add(m.msg.len())
}

/// 日志批发线程主循环.
///
/// 只读取一个 MPSC 通道 (`LogIn`), 维护内部缓冲区, 检查待写总条数与总大小.
/// 任一限制达到时丢弃日志并限速警告. 收到写入线程的 `Batch` 请求时, 清空内部缓冲
/// 并把整批日志发给写入线程.
pub(super) fn log_batch_worker(
    rx: mpsc::Receiver<LogIn>,
    w_tx: mpsc::SyncSender<WriterCmd>,
    count_limit: usize,
    byte_limit: usize,
    warn_interval_min: Duration,
    life: Weak<()>,
) {
    let mut buf = LogBuffer::new();
    // 是否已把一批日志交给写入线程, 但尚未收到下一次 Batch 请求确认写入完成.
    let mut writer_busy = false;
    // 写入线程是否已请求一批日志, 但当前缓冲区为空, 正在等待数据.
    let mut pending_batch = false;
    // 待回执的 flush 请求 (按到达顺序保存, 等待之前的日志全部写入).
    let mut pending_flush: Vec<oneshot::Sender<()>> = Vec::new();

    loop {
        match rx.recv_timeout(SHUTDOWN_POLL) {
            Ok(cmd) => match cmd {
                LogIn::Log(m) => {
                    let size = log_msg_size(&m);
                    if buf.len() >= count_limit {
                        warn_rate_limited(
                            "log-count-full",
                            "log buffer full (count); dropping log",
                            warn_interval_min,
                        );
                        continue;
                    }
                    if buf.bytes.saturating_add(size) > byte_limit {
                        warn_rate_limited(
                            "log-size-full",
                            "log buffer full (size); dropping log",
                            warn_interval_min,
                        );
                        continue;
                    }
                    buf.push(m, size);
                    if pending_batch {
                        // 写入线程正在等待数据, 立即把当前缓冲整批发过去.
                        match send_batch(&mut buf, &w_tx) {
                            Ok(()) => {
                                pending_batch = false;
                                writer_busy = true;
                            }
                            Err(()) => return,
                        }
                    }
                }
                LogIn::Batch => {
                    // 收到下一次请求表示写入线程已完成上一批写入.
                    writer_busy = false;
                    if buf.is_empty() {
                        // 缓冲区为空且写入线程空闲: 之前所有日志均已写入, 可以回执 flush.
                        ack_flushes(&mut pending_flush);
                        pending_batch = true;
                    } else {
                        match send_batch(&mut buf, &w_tx) {
                            Ok(()) => writer_busy = true,
                            Err(()) => return,
                        }
                    }
                }
                LogIn::Flush(ack) => {
                    if buf.is_empty() && !writer_busy {
                        // 没有待写日志, 也没有正在写入的批, 直接回执.
                        // 请求方可能已取消等待, 此时 receiver 已丢弃, 忽略发送失败.
                        let _ = ack.send(());
                    } else {
                        pending_flush.push(ack);
                    }
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // DbClient 全部释放后退出批发线程.
                if life.upgrade().is_none() {
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// 清空内部缓冲并发送给写入线程. 发送失败时返回 `Err`, 调用方退出线程.
fn send_batch(buf: &mut LogBuffer, w_tx: &mpsc::SyncSender<WriterCmd>) -> Result<(), ()> {
    let batch = buf.take();
    w_tx.send(WriterCmd::Batch(batch)).map_err(|_| ())
}

/// 回执所有待处理的 flush 请求.
fn ack_flushes(pending: &mut Vec<oneshot::Sender<()>>) {
    for ack in pending.drain(..) {
        // 请求方可能已取消等待, 此时 receiver 已丢弃, 忽略发送失败.
        let _ = ack.send(());
    }
}

/// 把日志查询结果行转换为 [`LogEntry`]
fn log_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LogEntry> {
    Ok(LogEntry {
        id: row.get(0)?,
        ts: row.get(1)?,
        level: row.get(2)?,
        unit: row.get(3)?,
        msg: row.get(4)?,
    })
}

/// 根据 level / unit 过滤条件构造日志查询的 WHERE 片段与对应参数.
///
/// 返回的片段不含 `WHERE` 关键字, 可为空字符串. 条件按 `level = ?` / `unit = ?`
/// 的顺序拼接, 参数顺序保持一致. 全部为等值条件, 可被 `idx_log_level` /
/// `idx_log_unit` 索引使用. 不用 `? IS NULL OR ...`, 避免索引失效.
fn log_conds(level: Option<&str>, unit: Option<&str>) -> (String, Vec<Box<dyn ToSql>>) {
    let mut conds: Vec<&str> = Vec::new();
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();

    if let Some(l) = level {
        conds.push("(level = ?)");
        params.push(Box::new(l.to_string()));
    }
    if let Some(u) = unit {
        conds.push("(unit = ?)");
        params.push(Box::new(u.to_string()));
    }

    (conds.join(" AND "), params)
}

/// 把自有参数列表转换为 rusqlite 接受的 `&dyn ToSql` 引用切片.
fn dyn_params(params: &[Box<dyn ToSql>]) -> Vec<&dyn ToSql> {
    params.iter().map(|p| p.as_ref() as &dyn ToSql).collect()
}
