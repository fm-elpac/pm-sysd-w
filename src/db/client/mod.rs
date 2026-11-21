//! 数据库客户端 (pub API) 与数据库读写/日志写入线程的启动.

use std::{
    sync::{Arc, mpsc},
    thread,
};

use tokio::sync::oneshot;

use crate::conf::limit::SysdLimit;

use super::{
    DbError, UnitState,
    logd::{DbLog, LogIn, WriterCmd, log_batch_worker, log_writer},
    rw::{DbRequest, DbRw, db_worker, recv_resp, send_req},
    util::Db,
};

/// 数据库客户端 (async 线程使用).
///
/// 克隆共享同一个数据库读写线程, 日志批发线程与日志写入线程的发送端.
#[derive(Clone)]
pub struct DbClient {
    /// 数据库读写线程的发送端
    pub(super) req_tx: mpsc::SyncSender<DbRequest>,
    /// 日志批发线程的发送端
    pub(super) log_tx: mpsc::SyncSender<LogIn>,
    /// 用于在最后一个客户端释放后通知日志线程退出
    _life: Arc<()>,
}

impl DbClient {
    /// 打开 (或创建) 数据库, 并启动数据库读写线程, 日志批发线程与日志写入线程
    pub fn open(path: &str, limit: &SysdLimit) -> Result<Self, DbError> {
        Self::open_impl(path, limit, false)
    }

    /// 打开 (或创建) 数据库, 日志表不创建索引 (写入优化模式), 并启动各数据库线程
    pub fn open_log_no_index(path: &str, limit: &SysdLimit) -> Result<Self, DbError> {
        Self::open_impl(path, limit, true)
    }

    /// 打开 (或创建) 数据库并启动各数据库线程的内部实现.
    ///
    /// `log_no_index` 为 `true` 时日志表不创建索引.
    fn open_impl(path: &str, limit: &SysdLimit, log_no_index: bool) -> Result<Self, DbError> {
        let db = if log_no_index {
            Db::open_log_no_index(path)?
        } else {
            Db::open(path)?
        };
        let rw = DbRw::new(db.clone());
        let log = DbLog::new(db);

        let (req_tx, req_rx) = mpsc::sync_channel::<DbRequest>(limit.db_channel_capacity_n);
        thread::Builder::new()
            .name("pm-sysd-w-db".to_string())
            .spawn(move || db_worker(rw, req_rx))
            .map_err(|e| DbError::Msg(format!("spawn db worker thread: {}", e)))?;

        // 日志消息 / 批发请求 / flush 请求 共用的输入通道.
        let (log_tx, log_rx) = mpsc::sync_channel::<LogIn>(limit.log_channel_capacity_n);
        // 批发线程把一批日志发给写入线程的通道 (容量 1, 一次只发一批).
        let (w_tx, w_rx) = mpsc::sync_channel::<WriterCmd>(1);

        // 最后一个 DbClient 释放后, 通过 Weak 引用通知日志线程退出.
        let life = Arc::new(());
        let writer_life = Arc::downgrade(&life);
        let batch_life = Arc::downgrade(&life);

        let log_line_byte = limit.log_line_byte;
        let warn_interval_min = limit.warn_interval_min;
        let count_limit = limit.log_pending_n;
        let byte_limit = limit.log_pending_byte;

        let writer_log_tx = log_tx.clone();
        thread::Builder::new()
            .name("pm-sysd-w-log".to_string())
            .spawn(move || {
                log_writer(
                    log,
                    writer_log_tx,
                    w_rx,
                    log_line_byte,
                    warn_interval_min,
                    writer_life,
                )
            })
            .map_err(|e| DbError::Msg(format!("spawn log writer thread: {}", e)))?;

        thread::Builder::new()
            .name("pm-sysd-w-log-batch".to_string())
            .spawn(move || {
                log_batch_worker(
                    log_rx,
                    w_tx,
                    count_limit,
                    byte_limit,
                    warn_interval_min,
                    batch_life,
                )
            })
            .map_err(|e| DbError::Msg(format!("spawn log batch thread: {}", e)))?;

        Ok(Self {
            req_tx,
            log_tx,
            _life: life,
        })
    }

    /// 读取 unit 表全部数据 (单次查询)
    pub async fn unit_list_all(&self) -> Result<Vec<UnitState>, DbError> {
        let (resp, rx) = oneshot::channel();
        send_req(&self.req_tx, DbRequest::UnitListAll { resp })?;
        recv_resp(rx).await
    }

    /// 写入一个 unit 的完整状态 (全部默认值时删除该行)
    pub async fn unit_write(&self, state: &UnitState) -> Result<(), DbError> {
        let (resp, rx) = oneshot::channel();
        send_req(
            &self.req_tx,
            DbRequest::UnitWrite {
                state: state.clone(),
                resp,
            },
        )?;
        recv_resp(rx).await
    }

    /// 对数据库执行 vacuum (由数据库读写线程处理)
    pub async fn vacuum(&self) -> Result<(), DbError> {
        let (resp, rx) = oneshot::channel();
        send_req(&self.req_tx, DbRequest::Vacuum { resp })?;
        recv_resp(rx).await
    }
}
