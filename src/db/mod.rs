//! 数据库 (sqlite) 相关功能的抽象/封装.
//!
//! pm-sysd-init 使用 sqlite 数据库存储持久状态 (例如 unit 是否 enable/mask), 以及
//! 日志数据. 数据库格式属于内部 API, 不作为外部稳定 API 提供.
//!
//! 数据库 schema 与版本/升级规则见 `doc/dev/db.md`.
//!
//! ## 线程模型
//!
//! sqlite 访问只发生在 2 个专用 (同步) 线程中:
//!
//! - 数据库读写线程: 处理 [`DbClient`] 通过 MPSC 发来的读写请求, 并把结果通过
//!   请求中携带的响应 MPSC 发回.
//! - 日志写入线程: 处理 `DbClient::log_append` 通过 MPSC 发来的日志写入请求.
//!
//! tokio 的 async 线程不直接访问数据库, 因此不会被 sqlite IO 阻塞.

use std::{
    fmt, fs,
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    thread,
};

use chrono::{SecondsFormat, Utc};
use rusqlite::{Connection, params};
use tokio::sync::mpsc as tmpsc;

mod error;
mod schema;
mod worker;

#[cfg(test)]
mod test;

pub use error::DbError;
pub use schema::{MAX_LOG_LEN, PROGRAM_NAME, SCHEMA_VERSION};

use schema::{SCHEMA_SQL, migrate};
use worker::{
    DbRequest, LogCmd, LogMsg, db_worker, log_worker, recv_resp, recv_resp_blocking, send_req,
};

/// 日志写入线程的缓冲区大小 (条)
const LOG_CHANNEL_CAPACITY: usize = 1000;

/// 数据库读写线程的请求缓冲区大小 (条)
const DB_CHANNEL_CAPACITY: usize = 1000;
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

/// 数据库客户端 (async 线程使用).
///
/// 克隆共享同一个数据库读写线程与日志写入线程的发送端.
#[derive(Clone)]
pub struct DbClient {
    /// 数据库读写线程的发送端
    req_tx: SyncSender<DbRequest>,
    /// 日志写入线程的发送端
    log_tx: SyncSender<LogCmd>,
}

impl DbClient {
    /// 打开 (或创建) 数据库, 并启动专用数据库读写线程与日志写入线程
    pub fn open(path: &str) -> Result<Self, DbError> {
        let db = Arc::new(Db::open(path)?);

        let (req_tx, req_rx) = mpsc::sync_channel::<DbRequest>(DB_CHANNEL_CAPACITY);
        let worker_db = db.clone();
        thread::Builder::new()
            .name("pm-sysd-w-db".to_string())
            .spawn(move || db_worker(worker_db, req_rx))
            .expect("spawn db worker thread");

        let (log_tx, log_rx) = mpsc::sync_channel::<LogCmd>(LOG_CHANNEL_CAPACITY);
        let log_db = db.clone();
        thread::Builder::new()
            .name("pm-sysd-w-log".to_string())
            .spawn(move || log_worker(log_db, log_rx))
            .expect("spawn log worker thread");

        Ok(Self { req_tx, log_tx })
    }

    /// 读取 meta 键值
    pub async fn meta_get(&self, key: &str) -> Result<Option<String>, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::MetaGet {
                key: key.to_string(),
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 写入 meta 键值 (覆盖)
    pub async fn meta_set(&self, key: &str, value: &str) -> Result<(), DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::MetaSet {
                key: key.to_string(),
                value: value.to_string(),
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 读取 unit 的 enable 状态 (不存在则 false)
    pub async fn unit_get_enabled(&self, name: &str) -> Result<bool, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::UnitGetEnabled {
                name: name.to_string(),
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 设置 unit 的 enable 状态
    pub async fn unit_set_enabled(&self, name: &str, enabled: bool) -> Result<(), DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::UnitSetEnabled {
                name: name.to_string(),
                enabled,
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 读取 unit 的 mask 状态 (不存在则 false)
    pub async fn unit_get_masked(&self, name: &str) -> Result<bool, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::UnitGetMasked {
                name: name.to_string(),
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 设置 unit 的 mask 状态
    pub async fn unit_set_masked(&self, name: &str, masked: bool) -> Result<(), DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::UnitSetMasked {
                name: name.to_string(),
                masked,
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 读取 全部 enable 的 unit 名称
    pub async fn unit_list_enabled(&self) -> Result<Vec<String>, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(&self.req_tx, DbRequest::UnitListEnabled { resp })?;
        recv_resp(&mut rx).await
    }

    /// 读取 全部 mask 的 unit 名称
    pub async fn unit_list_masked(&self) -> Result<Vec<String>, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(&self.req_tx, DbRequest::UnitListMasked { resp })?;
        recv_resp(&mut rx).await
    }

    /// 读取最近的若干条日志 (可选按 unit 过滤)
    pub async fn log_recent(
        &self,
        unit: Option<&str>,
        limit: u32,
    ) -> Result<Vec<LogEntry>, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::LogRecent {
                unit: unit.map(|s| s.to_string()),
                limit,
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 统计匹配条件的日志总量
    pub async fn log_total(&self, unit: Option<&str>) -> Result<u64, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::LogTotal {
                unit: unit.map(|s| s.to_string()),
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 读取日志分页 (按 id 倒序, 供上层计算游标).
    ///
    /// `cursor` 为 `Some(id)` 时, 只读取 `id < cursor` 的日志.
    pub async fn log_page_raw(
        &self,
        unit: Option<&str>,
        limit: u32,
        cursor: Option<i64>,
    ) -> Result<Vec<LogEntry>, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::LogPageRaw {
                unit: unit.map(|s| s.to_string()),
                limit,
                cursor,
                resp,
            },
        )?;
        recv_resp(&mut rx).await
    }

    /// 删除时间戳早于 `ts` 的日志, 返回删除条数 (同步阻塞, 供非 async 线程使用).
    pub fn log_delete_before_blocking(&self, ts: &str) -> Result<u64, DbError> {
        let (resp, mut rx) = tmpsc::channel(1);
        send_req(
            &self.req_tx,
            DbRequest::LogDeleteBefore {
                ts: ts.to_string(),
                resp,
            },
        )?;
        recv_resp_blocking(&mut rx)
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
    pub fn log_flush(&self) -> Result<(), DbError> {
        let (ack_tx, ack_rx) = mpsc::sync_channel::<()>(1);
        self.log_tx
            .send(LogCmd::Flush(ack_tx))
            .map_err(|_| DbError::Msg("log writer stopped".to_string()))?;
        ack_rx
            .recv()
            .map_err(|_| DbError::Msg("log writer stopped".to_string()))?;
        Ok(())
    }
}

/// 数据库连接 (封装 sqlite)
struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// 打开 (或创建) 数据库, 检查 schema 版本, 必要时自动升级.
    ///
    /// 打开流程 (详见 `doc/dev/db.md`):
    ///
    /// - 仅当 `pm_sysd_w_meta` 表不存在时 才创建 schema (新数据库).
    /// - 表已存在时 先读取 `program` / `schema_version` (不做任何写入).
    /// - 仅当 需要升级 (旧版本) 时 才执行迁移, 迁移封装在 单个事务 中.
    fn open(path: &str) -> Result<Self, DbError> {
        // 确保上级目录存在
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .map_err(|e| DbError::Sqlite(format!("create dir `{:?}`: {}", parent, e)))?;
            }
        }

        let conn = Connection::open(path)
            .map_err(|e| DbError::Sqlite(format!("open `{}`: {}", path, e)))?;
        let db = Self {
            conn: Mutex::new(conn),
        };

        if !db.meta_table_exists()? {
            // 新数据库: 创建 schema 并写入元信息
            db.create_schema()?;
            db.init_new_db()?;
            return Ok(db);
        }

        // 表已存在: 读取程序标识 / 版本, 不做写入
        match db.meta_get("program")? {
            Some(p) if p == PROGRAM_NAME => {}
            Some(p) => {
                return Err(DbError::Program(format!(
                    "database `{}` belongs to `{}`, not `{}`",
                    path, p, PROGRAM_NAME
                )));
            }
            None => {
                return Err(DbError::Corrupt(
                    "pm_sysd_w_meta 表存在但缺少 `program` 键".into(),
                ));
            }
        }

        let current = db
            .meta_get("schema_version")?
            .and_then(|v| v.parse::<u32>().ok())
            .ok_or_else(|| {
                DbError::Corrupt("pm_sysd_w_meta 表存在但缺少有效的 `schema_version` 键".into())
            })?;

        if current > SCHEMA_VERSION {
            return Err(DbError::Version(format!(
                "database schema version `{}` is newer than supported `{}`",
                current, SCHEMA_VERSION
            )));
        }

        if current < SCHEMA_VERSION {
            db.upgrade(current)?;
        }

        Ok(db)
    }

    /// 检查 `pm_sysd_w_meta` 表是否存在 (只读)
    fn meta_table_exists(&self) -> Result<bool, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'pm_sysd_w_meta'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| DbError::Sqlite(format!("meta_table_exists: {}", e)))?;
        Ok(n > 0)
    }

    /// 创建新数据库 schema
    fn create_schema(&self) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute_batch(SCHEMA_SQL)
            .map_err(|e| DbError::Sqlite(format!("create schema: {}", e)))
    }

    /// 初始化新建数据库的 meta
    fn init_new_db(&self) -> Result<(), DbError> {
        let now = now_iso();
        let version = env!("CARGO_PKG_VERSION");

        self.meta_set("program", PROGRAM_NAME)?;
        self.meta_set("schema_version", &SCHEMA_VERSION.to_string())?;
        self.meta_set("created_by_version", version)?;
        self.meta_set("last_upgrade_version", version)?;
        self.meta_set("created_at", &now)?;
        self.meta_set("updated_at", &now)?;
        Ok(())
    }

    /// 把数据库从 `current` 版本逐步升级到 [`SCHEMA_VERSION`].
    ///
    /// 全部迁移步骤与元信息更新封装在 单个事务 中, 原子完成.
    fn upgrade(&self, current: u32) -> Result<(), DbError> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn
            .transaction()
            .map_err(|e| DbError::Sqlite(format!("begin transaction: {}", e)))?;

        for target in (current + 1)..=SCHEMA_VERSION {
            migrate(&tx, target)?;
        }

        tx.execute(
            "INSERT INTO pm_sysd_w_meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![SCHEMA_VERSION.to_string()],
        )
        .map_err(|e| DbError::Sqlite(format!("upgrade schema_version: {}", e)))?;

        tx.execute(
            "INSERT INTO pm_sysd_w_meta (key, value) VALUES ('last_upgrade_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![env!("CARGO_PKG_VERSION")],
        )
        .map_err(|e| DbError::Sqlite(format!("upgrade last_upgrade_version: {}", e)))?;

        tx.execute(
            "INSERT INTO pm_sysd_w_meta (key, value) VALUES ('updated_at', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![now_iso()],
        )
        .map_err(|e| DbError::Sqlite(format!("upgrade updated_at: {}", e)))?;

        tx.commit()
            .map_err(|e| DbError::Sqlite(format!("commit transaction: {}", e)))?;
        Ok(())
    }

    /// 读取 meta 键值
    fn meta_get(&self, key: &str) -> Result<Option<String>, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare("SELECT value FROM pm_sysd_w_meta WHERE key = ?1")
            .map_err(|e| DbError::Sqlite(format!("meta_get: {}", e)))?;
        let mut rows = stmt
            .query_map(params![key], |row| row.get::<_, String>(0))
            .map_err(|e| DbError::Sqlite(format!("meta_get: {}", e)))?;
        match rows.next() {
            Some(v) => v
                .map(Some)
                .map_err(|e| DbError::Sqlite(format!("meta_get: {}", e))),
            None => Ok(None),
        }
    }

    /// 写入 meta 键值 (覆盖)
    fn meta_set(&self, key: &str, value: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO pm_sysd_w_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| DbError::Sqlite(format!("meta_set: {}", e)))?;
        Ok(())
    }

    /// 确保 unit 在 `unit` 表中存在 (不存在则插入默认行)
    fn unit_ensure(&self, name: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO unit (name, enabled, masked) VALUES (?1, 0, 0)
             ON CONFLICT(name) DO NOTHING",
            params![name],
        )
        .map_err(|e| DbError::Sqlite(format!("unit_ensure: {}", e)))?;
        Ok(())
    }

    /// 设置 unit 的 enable 状态
    fn unit_set_enabled(&self, name: &str, enabled: bool) -> Result<(), DbError> {
        self.unit_ensure(name)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE unit SET enabled = ?1 WHERE name = ?2",
            params![enabled as i32, name],
        )
        .map_err(|e| DbError::Sqlite(format!("unit_set_enabled: {}", e)))?;
        Ok(())
    }

    /// 读取 unit 的 enable 状态 (不存在则 false)
    fn unit_get_enabled(&self, name: &str) -> Result<bool, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare("SELECT enabled FROM unit WHERE name = ?1")
            .map_err(|e| DbError::Sqlite(format!("unit_get_enabled: {}", e)))?;
        let mut rows = stmt
            .query_map(params![name], |row| row.get::<_, i32>(0))
            .map_err(|e| DbError::Sqlite(format!("unit_get_enabled: {}", e)))?;
        match rows.next() {
            Some(v) => Ok(v.map(|n| n != 0).unwrap_or(false)),
            None => Ok(false),
        }
    }

    /// 设置 unit 的 mask 状态
    fn unit_set_masked(&self, name: &str, masked: bool) -> Result<(), DbError> {
        self.unit_ensure(name)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE unit SET masked = ?1 WHERE name = ?2",
            params![masked as i32, name],
        )
        .map_err(|e| DbError::Sqlite(format!("unit_set_masked: {}", e)))?;
        Ok(())
    }

    /// 读取 unit 的 mask 状态 (不存在则 false)
    fn unit_get_masked(&self, name: &str) -> Result<bool, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare("SELECT masked FROM unit WHERE name = ?1")
            .map_err(|e| DbError::Sqlite(format!("unit_get_masked: {}", e)))?;
        let mut rows = stmt
            .query_map(params![name], |row| row.get::<_, i32>(0))
            .map_err(|e| DbError::Sqlite(format!("unit_get_masked: {}", e)))?;
        match rows.next() {
            Some(v) => Ok(v.map(|n| n != 0).unwrap_or(false)),
            None => Ok(false),
        }
    }

    /// 读取 全部 enable 的 unit 名称
    fn unit_list_enabled(&self) -> Result<Vec<String>, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare("SELECT name FROM unit WHERE enabled != 0 ORDER BY name")
            .map_err(|e| DbError::Sqlite(format!("unit_list_enabled: {}", e)))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| DbError::Sqlite(format!("unit_list_enabled: {}", e)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| DbError::Sqlite(format!("unit_list_enabled: {}", e)))?);
        }
        Ok(out)
    }

    /// 读取 全部 mask 的 unit 名称
    fn unit_list_masked(&self) -> Result<Vec<String>, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare("SELECT name FROM unit WHERE masked != 0 ORDER BY name")
            .map_err(|e| DbError::Sqlite(format!("unit_list_masked: {}", e)))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| DbError::Sqlite(format!("unit_list_masked: {}", e)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| DbError::Sqlite(format!("unit_list_masked: {}", e)))?);
        }
        Ok(out)
    }

    /// 追加一条日志
    fn log_append(
        &self,
        ts: &str,
        level: &str,
        unit: Option<&str>,
        msg: &str,
    ) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO log (ts, level, unit, msg) VALUES (?1, ?2, ?3, ?4)",
            params![ts, level, unit, msg],
        )
        .map_err(|e| DbError::Sqlite(format!("log_append: {}", e)))?;
        Ok(())
    }

    /// 读取最近的若干条日志 (可选按 unit 过滤)
    fn log_recent(&self, unit: Option<&str>, limit: u32) -> Result<Vec<LogEntry>, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let sql = "SELECT id, ts, level, unit, msg FROM log
                   WHERE (?1 IS NULL OR unit = ?1) ORDER BY id DESC LIMIT ?2";

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| DbError::Sqlite(format!("log_recent: {}", e)))?;

        let rows = stmt
            .query_map(params![unit, limit as i64], |row| {
                Ok(LogEntry {
                    id: row.get(0)?,
                    ts: row.get(1)?,
                    level: row.get(2)?,
                    unit: row.get(3)?,
                    msg: row.get(4)?,
                })
            })
            .map_err(|e| DbError::Sqlite(format!("log_recent: {}", e)))?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| DbError::Sqlite(format!("log_recent: {}", e)))?);
        }
        out.reverse();
        Ok(out)
    }

    /// 统计匹配条件的日志总量
    fn log_total(&self, unit: Option<&str>) -> Result<u64, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM log WHERE (?1 IS NULL OR unit = ?1)",
                params![unit],
                |row| row.get(0),
            )
            .map_err(|e| DbError::Sqlite(format!("log_total: {}", e)))?;
        Ok(n as u64)
    }

    /// 读取日志分页 (按 id 倒序, 供上层计算游标).
    ///
    /// `cursor` 为 `Some(id)` 时, 只读取 `id < cursor` 的日志.
    /// `limit` 为最多读取条数; 上层通常会多取 1 条以判断是否还有下一页.
    fn log_page_raw(
        &self,
        unit: Option<&str>,
        limit: u32,
        cursor: Option<i64>,
    ) -> Result<Vec<LogEntry>, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let sql = "SELECT id, ts, level, unit, msg FROM log
                   WHERE (?1 IS NULL OR unit = ?1) AND (?2 IS NULL OR id < ?2)
                   ORDER BY id DESC LIMIT ?3";

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| DbError::Sqlite(format!("log_page_raw: {}", e)))?;

        let rows = stmt
            .query_map(params![unit, cursor, limit as i64], |row| {
                Ok(LogEntry {
                    id: row.get(0)?,
                    ts: row.get(1)?,
                    level: row.get(2)?,
                    unit: row.get(3)?,
                    msg: row.get(4)?,
                })
            })
            .map_err(|e| DbError::Sqlite(format!("log_page_raw: {}", e)))?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| DbError::Sqlite(format!("log_page_raw: {}", e)))?);
        }
        Ok(out)
    }

    /// 删除时间戳早于 `ts` 的日志, 返回删除条数 (日志自动清理)
    fn log_delete_before(&self, ts: &str) -> Result<u64, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let n = conn
            .execute("DELETE FROM log WHERE ts < ?1", params![ts])
            .map_err(|e| DbError::Sqlite(format!("log_delete_before: {}", e)))?;
        Ok(n as u64)
    }
}

impl fmt::Debug for Db {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Db").finish_non_exhaustive()
    }
}

/// 当前时间 (UTC, ISO 8601, 毫秒精度)
fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
