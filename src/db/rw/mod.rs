//! 数据库读写线程相关: `DbRw` 与请求/响应辅助逻辑.

#[cfg(test)]
mod test;

use std::sync::mpsc::{self, SyncSender};

use tokio::sync::oneshot;

use super::{
    DbError, UnitState,
    logd::{DbLog, LogPageRaw, LogPos},
    util::Db,
};

/// 数据库读写线程所需的操作.
pub(super) struct DbRw {
    db: Db,
}

impl DbRw {
    /// 新建 `DbRw` (内部持有共享的 `Db` 连接).
    pub(super) fn new(db: Db) -> Self {
        Self { db }
    }

    /// 读取 unit 表全部数据 (单次查询)
    fn unit_list_all(&self) -> Result<Vec<UnitState>, DbError> {
        self.db.with_tx(|tx| {
            let mut stmt = tx
                .prepare("SELECT name, enabled, masked FROM unit ORDER BY name")
                .map_err(|e| DbError::Sqlite(format!("unit_list_all: {}", e)))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(UnitState {
                        name: row.get(0)?,
                        enabled: row.get::<_, i64>(1)? != 0,
                        masked: row.get::<_, i64>(2)? != 0,
                    })
                })
                .map_err(|e| DbError::Sqlite(format!("unit_list_all: {}", e)))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(|e| DbError::Sqlite(format!("unit_list_all: {}", e)))?);
            }
            Ok(out)
        })
    }

    /// 写入一个 unit 的完整状态.
    ///
    /// 全部状态为默认值 (enabled=false 且 masked=false) 时删除此行,
    /// 避免 unit 表无限膨胀.
    fn unit_write(&self, state: &UnitState) -> Result<(), DbError> {
        self.db.with_tx(|tx| {
            if !state.enabled && !state.masked {
                tx.execute("DELETE FROM unit WHERE name = ?1", rusqlite::params![state.name])
                    .map_err(|e| DbError::Sqlite(format!("unit_write: {}", e)))?;
            } else {
                tx.execute(
                    "INSERT INTO unit (name, enabled, masked) VALUES (?1, ?2, ?3)
                     ON CONFLICT(name) DO UPDATE SET enabled = excluded.enabled, masked = excluded.masked",
                    rusqlite::params![state.name, state.enabled as i32, state.masked as i32],
                )
                .map_err(|e| DbError::Sqlite(format!("unit_write: {}", e)))?;
            }
            Ok(())
        })
    }

    /// 对数据库执行 vacuum (sqlite `VACUUM`).
    ///
    /// sqlite 不允许 `VACUUM` 在事务内执行, 因此这是唯一不创建事务的操作.
    /// 仍然只锁数据库一次.
    fn vacuum(&self) -> Result<(), DbError> {
        let conn = self.db.lock_conn();
        conn.execute_batch("VACUUM;")
            .map_err(|e| DbError::Sqlite(format!("vacuum: {}", e)))
    }
}

/// 发给数据库读写线程的请求
pub(super) enum DbRequest {
    /// 读取 unit 表全部数据
    UnitListAll {
        resp: oneshot::Sender<Result<Vec<UnitState>, DbError>>,
    },
    /// 写入一个 unit 的完整状态
    UnitWrite {
        state: UnitState,
        resp: oneshot::Sender<Result<(), DbError>>,
    },
    /// 对数据库执行 vacuum
    Vacuum {
        resp: oneshot::Sender<Result<(), DbError>>,
    },
    /// 读取日志分页 (按请求方向排序), 同时返回匹配条件的日志总量
    LogPage {
        level: Option<String>,
        unit: Option<String>,
        asc: bool,
        pos: LogPos,
        limit: u32,
        max_msg_bytes: usize,
        resp: oneshot::Sender<Result<LogPageRaw, DbError>>,
    },
    /// 删除时间戳早于 `ts` 的日志
    LogDeleteBefore {
        ts: String,
        resp: oneshot::Sender<Result<u64, DbError>>,
    },
}

/// 把请求投递给数据库读写线程 (非阻塞)
pub(super) fn send_req(tx: &SyncSender<DbRequest>, req: DbRequest) -> Result<(), DbError> {
    tx.try_send(req).map_err(|e| match e {
        mpsc::TrySendError::Full(_) => DbError::Busy("db request buffer full".into()),
        mpsc::TrySendError::Disconnected(_) => DbError::Msg("db worker stopped".into()),
    })
}

/// 等待数据库读写线程的响应 (async)
pub(super) async fn recv_resp<T>(rx: oneshot::Receiver<Result<T, DbError>>) -> Result<T, DbError> {
    match rx.await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(DbError::Msg("db worker stopped".into())),
    }
}

/// 数据库读写线程主循环
pub(super) fn db_worker(rw: DbRw, rx: mpsc::Receiver<DbRequest>) {
    let log = DbLog::new(rw.db.clone());
    while let Ok(req) = rx.recv() {
        match req {
            DbRequest::UnitListAll { resp } => send_resp(resp, rw.unit_list_all()),
            DbRequest::UnitWrite { state, resp } => send_resp(resp, rw.unit_write(&state)),
            DbRequest::Vacuum { resp } => send_resp(resp, rw.vacuum()),
            DbRequest::LogPage {
                level,
                unit,
                asc,
                pos,
                limit,
                max_msg_bytes,
                resp,
            } => send_resp(
                resp,
                log.log_page(
                    level.as_deref(),
                    unit.as_deref(),
                    asc,
                    pos,
                    limit,
                    max_msg_bytes,
                ),
            ),
            DbRequest::LogDeleteBefore { ts, resp } => send_resp(resp, log.log_delete_before(&ts)),
        }
    }
}

/// 把结果通过响应 oneshot 发回
fn send_resp<T>(resp: oneshot::Sender<Result<T, DbError>>, value: Result<T, DbError>) {
    // 调用方已放弃等待 (例如 async 任务被取消) 时 receiver 已丢弃,
    // 此时直接丢弃结果即可, 不影响数据库状态.
    let _ = resp.send(value);
}
