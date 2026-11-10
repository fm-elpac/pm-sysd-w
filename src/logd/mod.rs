//! 日志 核心逻辑 (类似 journald, 但简化实现).
//!
//! 日志数据 存入 pm-sysd-init 的 sqlite 数据库.
//!
//! 写入采用 专用 (同步) 线程 + MPSC: 发送方 无阻塞 (`try_send`), 接收方 使用较大
//! 缓冲区 (1000 条) 缓存 来不及写入的日志, 避免 日志写入 阻塞 主逻辑.

mod error;
#[cfg(test)]
mod test;

pub use error::LogdError;

use std::sync::{
    Arc,
    mpsc::{self, SyncSender},
};

use chrono::{SecondsFormat, Utc};

use crate::db::{Db, LogEntry};

/// 日志写入线程 的 缓冲区大小 (条).
const LOG_CHANNEL_CAPACITY: usize = 1000;

/// 一条 待写入 的日志.
struct LogMsg {
    ts: String,
    level: String,
    unit: Option<String>,
    msg: String,
}

/// 发给 日志写入线程 的命令.
enum LogCmd {
    /// 写入一条日志.
    Write(LogMsg),
    /// 请求 flush (写入线程 处理完之前所有 Write 后 回执).
    Flush(SyncSender<()>),
}

/// 日志记录器.
///
/// 克隆 共享 同一个 日志写入线程 的 发送端 与 数据库 读取端.
#[derive(Clone)]
pub struct Logd {
    /// 数据库 (读取端).
    db: Arc<Db>,
    /// 日志写入线程 的 发送端.
    tx: SyncSender<LogCmd>,
}

impl Logd {
    /// 创建日志记录器, 并 启动 专用 日志写入线程.
    pub fn new(db: Arc<Db>) -> Self {
        let (tx, rx) = mpsc::sync_channel::<LogCmd>(LOG_CHANNEL_CAPACITY);
        let writer_db = db.clone();
        let _ = std::thread::Builder::new()
            .name("pm-sysd-w-log".to_string())
            .spawn(move || {
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        LogCmd::Write(m) => {
                            if let Err(e) =
                                writer_db.log_append(&m.ts, &m.level, m.unit.as_deref(), &m.msg)
                            {
                                log::warn!("log write: {}", e);
                            }
                        }
                        LogCmd::Flush(ack) => {
                            let _ = ack.send(());
                        }
                    }
                }
            });
        Self { db, tx }
    }

    /// 写入一条日志 (级别 `info`, 无关联 unit).
    pub fn write(&mut self, msg: &str) -> Result<(), LogdError> {
        self.write_with("info", None, msg)
    }

    /// 写入一条日志 (指定 级别 / 关联 unit).
    ///
    /// 无阻塞: 仅把日志 投递到 写入线程 的 缓冲区, 不等待 数据库 写入完成.
    pub fn write_with(&self, level: &str, unit: Option<&str>, msg: &str) -> Result<(), LogdError> {
        let ts = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let cmd = LogCmd::Write(LogMsg {
            ts,
            level: level.to_string(),
            unit: unit.map(|s| s.to_string()),
            msg: msg.to_string(),
        });
        match self.tx.try_send(cmd) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => Err(LogdError::Busy("log buffer full".to_string())),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                Err(LogdError::Msg("log writer stopped".to_string()))
            }
        }
    }

    /// 等待 之前投递的 日志 全部写入数据库 (用于 优雅退出 与 测试).
    pub fn flush(&self) -> Result<(), LogdError> {
        let (ack_tx, ack_rx) = mpsc::sync_channel::<()>(1);
        self.tx
            .send(LogCmd::Flush(ack_tx))
            .map_err(|_| LogdError::Msg("log writer stopped".to_string()))?;
        ack_rx
            .recv()
            .map_err(|_| LogdError::Msg("log writer stopped".to_string()))?;
        Ok(())
    }

    /// 读取 最近的 若干条日志 (可选 按 unit 过滤).
    pub fn recent(&self, unit: Option<&str>, limit: u32) -> Result<Vec<String>, LogdError> {
        Ok(self
            .recent_entries(unit, limit)?
            .into_iter()
            .map(|e| match e.unit {
                Some(u) => format!("{} [{}] {}: {}", e.ts, e.level, u, e.msg),
                None => format!("{} [{}] {}", e.ts, e.level, e.msg),
            })
            .collect())
    }

    /// 读取 最近的 若干条日志 (结构化, 可选 按 unit 过滤).
    pub fn recent_entries(
        &self,
        unit: Option<&str>,
        limit: u32,
    ) -> Result<Vec<LogEntry>, LogdError> {
        self.db
            .log_recent(unit, limit)
            .map_err(|e| LogdError::Db(e.to_string()))
    }
}
