//! 数据库后台线程与请求/响应辅助逻辑.

use std::sync::{
    Arc,
    mpsc::{self, SyncSender},
};

use tokio::sync::oneshot;

use super::{Db, DbError, LogPageRaw, LogPos, UnitState};

/// 发给数据库读写线程的请求
pub enum DbRequest {
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
pub fn send_req(tx: &SyncSender<DbRequest>, req: DbRequest) -> Result<(), DbError> {
    tx.try_send(req).map_err(|e| match e {
        mpsc::TrySendError::Full(_) => DbError::Busy("db request buffer full".into()),
        mpsc::TrySendError::Disconnected(_) => DbError::Msg("db worker stopped".into()),
    })
}

/// 等待数据库读写线程的响应 (async)
pub async fn recv_resp<T>(rx: oneshot::Receiver<Result<T, DbError>>) -> Result<T, DbError> {
    match rx.await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(DbError::Msg("db worker stopped".into())),
    }
}

/// 数据库读写线程主循环
pub fn db_worker(db: Arc<Db>, rx: mpsc::Receiver<DbRequest>) {
    while let Ok(req) = rx.recv() {
        match req {
            DbRequest::UnitListAll { resp } => send_resp(resp, db.unit_list_all()),
            DbRequest::UnitWrite { state, resp } => send_resp(resp, db.unit_write(&state)),
            DbRequest::Vacuum { resp } => send_resp(resp, db.vacuum()),
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
                db.log_page(
                    level.as_deref(),
                    unit.as_deref(),
                    asc,
                    pos,
                    limit,
                    max_msg_bytes,
                ),
            ),
            DbRequest::LogDeleteBefore { ts, resp } => send_resp(resp, db.log_delete_before(&ts)),
        }
    }
}

/// 把结果通过响应 oneshot 发回
fn send_resp<T>(resp: oneshot::Sender<Result<T, DbError>>, value: Result<T, DbError>) {
    // 调用方已放弃等待 (例如 async 任务被取消) 时 receiver 已丢弃,
    // 此时直接丢弃结果即可, 不影响数据库状态.
    let _ = resp.send(value);
}
