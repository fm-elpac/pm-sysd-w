//! 日志相关的数据库读写与后台日志写入线程.

use std::sync::{Arc, mpsc};

use log::warn;
use rusqlite::ToSql;
use tokio::sync::{mpsc as tmpsc, oneshot};

use crate::logd::{MAX_LOG_LEN, warn_rate_limited};

use super::{
    Db, DbClient, DbError,
    worker::{DbRequest, recv_resp, send_req},
};

/// 日志写入线程的缓冲区大小 (条)
pub const LOG_CHANNEL_CAPACITY: usize = 1000;

/// 一条日志记录.
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

/// 日志分页原始查询结果.
#[derive(Debug, Clone)]
pub struct LogPageRaw {
    /// 查询到的日志 (按 id 倒序)
    pub entries: Vec<LogEntry>,
    /// 是否因累计日志内容长度达到上限而提前停止 (仍有更多匹配日志)
    pub size_limited: bool,
}

/// 一条待写入的日志
pub struct LogMsg {
    pub ts: String,
    pub level: String,
    pub unit: Option<String>,
    pub msg: String,
}

/// 发给日志写入线程的命令
pub enum LogCmd {
    /// 写入一条日志
    Write(LogMsg),
    /// 请求 flush (写入线程处理完之前所有 Write 后回执)
    Flush(oneshot::Sender<()>),
}

impl DbClient {
    /// 统计匹配条件的日志总量 (可选按 level / unit 过滤)
    pub async fn log_total(&self, level: Option<&str>, unit: Option<&str>) -> Result<u64, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::LogTotal {
                level: level.map(|s| s.to_string()),
                unit: unit.map(|s| s.to_string()),
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 读取日志分页 (按 id 倒序, 供上层计算游标).
    ///
    /// `cursor` 为 `Some(id)` 时, 只读取 `id < cursor` 的日志.
    /// `max_msg_bytes` 为累计日志内容 (msg) 长度上限, 超出则提前停止,
    /// 避免一次读取过大.
    /// 始终至少返回 1 条 (若存在匹配日志).
    pub async fn log_page_raw(
        &self,
        level: Option<&str>,
        unit: Option<&str>,
        limit: u32,
        cursor: Option<i64>,
        max_msg_bytes: usize,
    ) -> Result<LogPageRaw, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::LogPageRaw {
                level: level.map(|s| s.to_string()),
                unit: unit.map(|s| s.to_string()),
                limit,
                cursor,
                max_msg_bytes,
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 删除时间戳早于 `ts` 的日志, 返回删除条数.
    pub async fn log_delete_before(&self, ts: &str) -> Result<u64, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::LogDeleteBefore {
                ts: ts.to_string(),
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 追加一条日志 (非阻塞, 仅投递到日志写入线程的缓冲区)
    pub fn log_append(
        &self,
        ts: &str,
        level: &str,
        unit: Option<&str>,
        msg: &str,
    ) -> Result<(), DbError> {
        let cmd = LogCmd::Write(LogMsg {
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
        // 用 spawn_blocking 执行阻塞的 send, 避免阻塞 async 线程.
        let send_res = tokio::task::spawn_blocking(move || {
            tx.send(LogCmd::Flush(ack_tx))
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

impl Db {
    /// 追加一条日志
    pub(super) fn log_append(
        &self,
        ts: &str,
        level: &str,
        unit: Option<&str>,
        msg: &str,
    ) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO log (ts, level, unit, msg) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ts, level, unit, msg],
        )
        .map_err(|e| DbError::Sqlite(format!("log_append: {}", e)))?;
        Ok(())
    }

    /// 统计匹配条件的日志总量 (可选按 level / unit 过滤)
    pub(super) fn log_total(
        &self,
        level: Option<&str>,
        unit: Option<&str>,
    ) -> Result<u64, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let (where_sql, params) = log_filter(level, unit, None);
        let sql = format!("SELECT COUNT(*) FROM log{}", where_sql);
        let n: i64 = conn
            .query_row(&sql, dyn_params(&params).as_slice(), |row| row.get(0))
            .map_err(|e| DbError::Sqlite(format!("log_total: {}", e)))?;
        Ok(n as u64)
    }

    /// 读取日志分页 (按 id 倒序, 供上层计算游标).
    ///
    /// `cursor` 为 `Some(id)` 时, 只读取 `id < cursor` 的日志.
    /// `limit` 为最多读取条数; 上层通常会多取 1 条以判断是否还有下一页.
    /// `max_msg_bytes` 为累计日志内容 (msg) 长度上限, 超出则提前停止.
    pub(super) fn log_page_raw(
        &self,
        level: Option<&str>,
        unit: Option<&str>,
        limit: u32,
        cursor: Option<i64>,
        max_msg_bytes: usize,
    ) -> Result<LogPageRaw, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let (where_sql, params) = log_filter(level, unit, cursor);
        let sql = format!(
            "SELECT id, ts, level, unit, msg FROM log{} ORDER BY id DESC LIMIT ?",
            where_sql
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| DbError::Sqlite(format!("log_page_raw: {}", e)))?;

        let limit = limit as i64;
        let mut param_refs = dyn_params(&params);
        param_refs.push(&limit);
        let rows = stmt
            .query_map(param_refs.as_slice(), log_from_row)
            .map_err(|e| DbError::Sqlite(format!("log_page_raw: {}", e)))?;

        let mut out = Vec::new();
        let mut total_bytes = 0usize;
        let mut size_limited = false;
        for r in rows {
            let e = r.map_err(|e| DbError::Sqlite(format!("log_page_raw: {}", e)))?;
            // 始终至少返回 1 条; 之后按累计内容长度提前停止.
            if !out.is_empty() && total_bytes.saturating_add(e.msg.len()) > max_msg_bytes {
                size_limited = true;
                break;
            }
            total_bytes = total_bytes.saturating_add(e.msg.len());
            out.push(e);
        }
        Ok(LogPageRaw {
            entries: out,
            size_limited,
        })
    }

    /// 删除时间戳早于 `ts` 的日志, 返回删除条数 (日志自动清理)
    pub(super) fn log_delete_before(&self, ts: &str) -> Result<u64, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n = conn
            .execute("DELETE FROM log WHERE ts < ?1", rusqlite::params![ts])
            .map_err(|e| DbError::Sqlite(format!("log_delete_before: {}", e)))?;
        Ok(n as u64)
    }
}

/// 日志写入线程主循环
pub fn log_worker(db: Arc<Db>, rx: mpsc::Receiver<LogCmd>) {
    while let Ok(cmd) = rx.recv() {
        match cmd {
            LogCmd::Write(m) => {
                // 防御性检查: 正常情况下发送方已拆分, 不应出现超长日志
                if m.msg.len() > MAX_LOG_LEN {
                    warn_rate_limited("log-over-long", "log writer: dropping over-long log");
                    continue;
                }
                if let Err(e) = db.log_append(&m.ts, &m.level, m.unit.as_deref(), &m.msg) {
                    warn!("log write: {}", e);
                }
            }
            LogCmd::Flush(ack) => {
                ack.send(()).expect("log flush ack receiver dropped");
            }
        }
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

/// 根据 level / unit / cursor 过滤条件构造日志查询的 WHERE 子句与对应参数.
///
/// 条件按 `level = ?` / `unit = ?` / `id < ?` 的顺序拼接, 参数顺序保持一致.
/// 全部为等值 (或范围) 条件, 可被 `idx_log_level` / `idx_log_unit` 索引使用;
/// 不用 `? IS NULL OR ...`, 避免索引失效.
fn log_filter(
    level: Option<&str>,
    unit: Option<&str>,
    cursor: Option<i64>,
) -> (String, Vec<Box<dyn ToSql>>) {
    let mut conds: Vec<&str> = Vec::new();
    let mut params: Vec<Box<dyn ToSql>> = Vec::new();

    if let Some(l) = level {
        conds.push("level = ?");
        params.push(Box::new(l.to_string()));
    }
    if let Some(u) = unit {
        conds.push("unit = ?");
        params.push(Box::new(u.to_string()));
    }
    if let Some(c) = cursor {
        conds.push("id < ?");
        params.push(Box::new(c));
    }

    let where_sql = if conds.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conds.join(" AND "))
    };
    (where_sql, params)
}

/// 把自有参数列表转换为 rusqlite 接受的 `&dyn ToSql` 引用切片.
fn dyn_params(params: &[Box<dyn ToSql>]) -> Vec<&dyn ToSql> {
    params.iter().map(|p| p.as_ref() as &dyn ToSql).collect()
}
