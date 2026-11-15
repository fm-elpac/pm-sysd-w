//! 数据库后台线程与 请求/响应 辅助逻辑.

use std::sync::{
    Arc,
    mpsc::{self, SyncSender},
};

use tokio::sync::mpsc as tmpsc;

use super::{Db, DbError, LogPageRaw};

/// 发给数据库读写线程的请求
pub enum DbRequest {
    /// 读取 meta 键值
    MetaGet {
        key: String,
        resp: tmpsc::Sender<Result<Option<String>, DbError>>,
    },
    /// 写入 meta 键值
    MetaSet {
        key: String,
        value: String,
        resp: tmpsc::Sender<Result<(), DbError>>,
    },
    /// 对数据库执行 vacuum
    Vacuum {
        resp: tmpsc::Sender<Result<(), DbError>>,
    },
    /// 设置 unit enable 状态
    UnitSetEnabled {
        name: String,
        enabled: bool,
        resp: tmpsc::Sender<Result<(), DbError>>,
    },
    /// 设置 unit mask 状态
    UnitSetMasked {
        name: String,
        masked: bool,
        resp: tmpsc::Sender<Result<(), DbError>>,
    },
    /// 读取全部 enable 的 unit 名称
    UnitListEnabled {
        resp: tmpsc::Sender<Result<Vec<String>, DbError>>,
    },
    /// 读取全部 mask 的 unit 名称
    UnitListMasked {
        resp: tmpsc::Sender<Result<Vec<String>, DbError>>,
    },
    /// 统计匹配条件的日志总量
    LogTotal {
        level: Option<String>,
        unit: Option<String>,
        resp: tmpsc::Sender<Result<u64, DbError>>,
    },
    /// 读取日志分页 (按 id 倒序)
    LogPageRaw {
        level: Option<String>,
        unit: Option<String>,
        limit: u32,
        cursor: Option<i64>,
        max_msg_bytes: usize,
        resp: tmpsc::Sender<Result<LogPageRaw, DbError>>,
    },
    /// 删除时间戳早于 `ts` 的日志
    LogDeleteBefore {
        ts: String,
        resp: tmpsc::Sender<Result<u64, DbError>>,
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
pub async fn recv_resp<T>(rx: &mut tmpsc::Receiver<Result<T, DbError>>) -> Result<T, DbError> {
    match rx.recv().await {
        Some(Ok(v)) => Ok(v),
        Some(Err(e)) => Err(e),
        None => Err(DbError::Msg("db worker stopped".into())),
    }
}

/// 数据库读写线程主循环
pub fn db_worker(db: Arc<Db>, rx: mpsc::Receiver<DbRequest>) {
    while let Ok(req) = rx.recv() {
        match req {
            DbRequest::MetaGet { key, resp } => send_resp(resp, db.meta_get(&key)),
            DbRequest::MetaSet { key, value, resp } => send_resp(resp, db.meta_set(&key, &value)),
            DbRequest::Vacuum { resp } => send_resp(resp, db.vacuum()),
            DbRequest::UnitSetEnabled {
                name,
                enabled,
                resp,
            } => send_resp(resp, db.unit_set_enabled(&name, enabled)),
            DbRequest::UnitSetMasked { name, masked, resp } => {
                send_resp(resp, db.unit_set_masked(&name, masked))
            }
            DbRequest::UnitListEnabled { resp } => send_resp(resp, db.unit_list_enabled()),
            DbRequest::UnitListMasked { resp } => send_resp(resp, db.unit_list_masked()),
            DbRequest::LogTotal { level, unit, resp } => {
                send_resp(resp, db.log_total(level.as_deref(), unit.as_deref()))
            }
            DbRequest::LogPageRaw {
                level,
                unit,
                limit,
                cursor,
                max_msg_bytes,
                resp,
            } => send_resp(
                resp,
                db.log_page_raw(
                    level.as_deref(),
                    unit.as_deref(),
                    limit,
                    cursor,
                    max_msg_bytes,
                ),
            ),
            DbRequest::LogDeleteBefore { ts, resp } => send_resp(resp, db.log_delete_before(&ts)),
        }
    }
}

/// 把结果通过响应 MPSC 发回
fn send_resp<T: std::fmt::Debug>(
    resp: tmpsc::Sender<Result<T, DbError>>,
    value: Result<T, DbError>,
) {
    resp.try_send(value).expect("db response receiver dropped");
}
