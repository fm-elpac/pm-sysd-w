//! 数据库后台线程与 请求/响应 辅助逻辑.

use std::{
    sync::{
        Arc, Mutex, OnceLock,
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};

use log::warn;
use tokio::sync::mpsc as tmpsc;

use super::{Db, DbError, LogEntry, MAX_LOG_LEN};

/// 丢弃日志警告的最小间隔 (速率限制)
const WARN_RATE_LIMIT: Duration = Duration::from_secs(1);

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
    Flush(SyncSender<()>),
}

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
    /// 读取 unit enable 状态
    UnitGetEnabled {
        name: String,
        resp: tmpsc::Sender<Result<bool, DbError>>,
    },
    /// 设置 unit enable 状态
    UnitSetEnabled {
        name: String,
        enabled: bool,
        resp: tmpsc::Sender<Result<(), DbError>>,
    },
    /// 读取 unit mask 状态
    UnitGetMasked {
        name: String,
        resp: tmpsc::Sender<Result<bool, DbError>>,
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
    /// 读取最近的若干条日志
    LogRecent {
        unit: Option<String>,
        limit: u32,
        resp: tmpsc::Sender<Result<Vec<LogEntry>, DbError>>,
    },
    /// 统计匹配条件的日志总量
    LogTotal {
        unit: Option<String>,
        resp: tmpsc::Sender<Result<u64, DbError>>,
    },
    /// 读取日志分页 (按 id 倒序)
    LogPageRaw {
        unit: Option<String>,
        limit: u32,
        cursor: Option<i64>,
        resp: tmpsc::Sender<Result<Vec<LogEntry>, DbError>>,
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

/// 等待数据库读写线程的响应 (同步阻塞)
pub fn recv_resp_blocking<T>(rx: &mut tmpsc::Receiver<Result<T, DbError>>) -> Result<T, DbError> {
    match rx.blocking_recv() {
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
            DbRequest::UnitGetEnabled { name, resp } => send_resp(resp, db.unit_get_enabled(&name)),
            DbRequest::UnitSetEnabled {
                name,
                enabled,
                resp,
            } => send_resp(resp, db.unit_set_enabled(&name, enabled)),
            DbRequest::UnitGetMasked { name, resp } => send_resp(resp, db.unit_get_masked(&name)),
            DbRequest::UnitSetMasked { name, masked, resp } => {
                send_resp(resp, db.unit_set_masked(&name, masked))
            }
            DbRequest::UnitListEnabled { resp } => send_resp(resp, db.unit_list_enabled()),
            DbRequest::UnitListMasked { resp } => send_resp(resp, db.unit_list_masked()),
            DbRequest::LogRecent { unit, limit, resp } => {
                send_resp(resp, db.log_recent(unit.as_deref(), limit))
            }
            DbRequest::LogTotal { unit, resp } => send_resp(resp, db.log_total(unit.as_deref())),
            DbRequest::LogPageRaw {
                unit,
                limit,
                cursor,
                resp,
            } => send_resp(resp, db.log_page_raw(unit.as_deref(), limit, cursor)),
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

/// 日志写入线程主循环
pub fn log_worker(db: Arc<Db>, rx: mpsc::Receiver<LogCmd>) {
    while let Ok(cmd) = rx.recv() {
        match cmd {
            LogCmd::Write(m) => {
                // 防御性检查: 正常情况下发送方已拆分, 不应出现超长日志
                if m.msg.len() > MAX_LOG_LEN {
                    warn_rate_limited("log writer: dropping over-long log");
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

/// 限速警告: 同一条消息至少间隔 [`WARN_RATE_LIMIT`] 才输出一次
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
