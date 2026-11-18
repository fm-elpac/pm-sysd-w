//! 数据库 (sqlite) 相关功能的抽象/封装.
//!
//! pm-sysd-init 使用 sqlite 数据库存储持久状态 (例如 unit 是否 enable/mask),
//! 以及日志数据. 数据库格式属于内部 API, 不作为外部稳定 API 提供.
//!
//! 数据库 schema 与版本/升级规则见 `doc/dev/db.md`.
//!
//! ## 线程模型
//!
//! sqlite 访问只发生在 2 个专用 (同步) 线程中:
//!
//! - 数据库读写线程: 处理 [`DbClient`] 通过 MPSC 发来的读写请求,
//!   并把结果通过请求中携带的响应 oneshot 发回.
//!
//! - 日志写入线程: 处理 `DbClient::log_append` 通过 MPSC 发来的日志写入请求.
//!
//! tokio 的 async 线程不直接访问数据库, 因此不会被 sqlite IO 阻塞.
//!
//! ## 事务约定
//!
//! 每个 pub API 的单次操作 (打开 / 读 / 写) 都通过 [`Db::with_tx`] 处理:
//! 锁数据库连接一次, 创建事务一次, 全部 sqlite 读写都在该事务内完成.
//!
//! 唯一的例外是 `vacuum`, 因为 sqlite 不允许 `VACUUM` 在事务内执行.

use std::{
    fmt, fs,
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    thread,
};

use rusqlite::{Connection, Transaction, params};
use tokio::sync::oneshot;

mod error;
mod logd;
mod schema;
mod worker;

#[cfg(test)]
mod test;

pub use error::DbError;
pub use logd::{LogEntry, LogPageRaw, LogPos};
pub use schema::{PROGRAM_NAME, SCHEMA_VERSION};

use logd::{LOG_CHANNEL_CAPACITY, LogCmd, log_worker};
use schema::{SCHEMA_SQL, migrate};
use worker::{DbRequest, db_worker, recv_resp, send_req};

/// 数据库读写线程的请求缓冲区大小 (条)
const DB_CHANNEL_CAPACITY: usize = 1000;

/// 单个 unit 的持久状态 (unit 表中的一行)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitState {
    /// unit 名称
    pub name: String,
    /// 是否 enable
    pub enabled: bool,
    /// 是否 mask
    pub masked: bool,
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

/// 数据库连接 (封装 sqlite)
struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// 打开 (或创建) 数据库, 检查 schema 版本, 必要时自动初始化/升级.
    ///
    /// 版本检查, 初始化, 升级, 全部封装在 **单个事务** 中 (见 [`Self::init_or_upgrade`]).
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

        db.init_or_upgrade(path)?;
        Ok(db)
    }

    /// 在单个事务中完成: 版本检查, 初始化, 升级.
    ///
    /// - 仅当 `pm_sysd_w_meta` 表不存在时才创建 schema (新数据库).
    ///   建表 / 建索引 / 初始元信息, 都在该事务中.
    /// - 表已存在时先读取 `program` / `schema_version` (只读).
    /// - 仅当需要升级 (旧版本) 时才执行迁移.
    fn init_or_upgrade(&self, path: &str) -> Result<(), DbError> {
        self.with_tx(|tx| {
            if !meta_table_exists(tx)? {
                init_new_db(tx)?;
                return Ok(());
            }

            let program = meta_get_on(tx, "program")?.ok_or_else(|| {
                DbError::Corrupt("pm_sysd_w_meta 表存在但缺少 `program` 键".into())
            })?;
            if program != PROGRAM_NAME {
                return Err(DbError::Program(format!(
                    "database `{}` belongs to `{}`, not `{}`",
                    path, program, PROGRAM_NAME
                )));
            }

            let current = meta_get_on(tx, "schema_version")?
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
                upgrade(tx, current)?;
            }
            Ok(())
        })
    }

    /// 锁数据库连接一次并创建单个事务, 执行 `f`, 成功后提交.
    ///
    /// `f` 返回错误时, 事务随 `tx` 丢弃而回滚.
    fn with_tx<T>(&self, f: impl FnOnce(&Transaction) -> Result<T, DbError>) -> Result<T, DbError> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tx = conn
            .transaction()
            .map_err(|e| DbError::Sqlite(format!("begin transaction: {}", e)))?;

        match f(&tx) {
            Ok(v) => {
                tx.commit()
                    .map_err(|e| DbError::Sqlite(format!("commit transaction: {}", e)))?;
                Ok(v)
            }
            Err(e) => Err(e),
        }
    }

    /// 读取 meta 键值 (仅测试使用)
    #[cfg(test)]
    fn meta_get(&self, key: &str) -> Result<Option<String>, DbError> {
        self.with_tx(|tx| meta_get_on(tx, key))
    }

    /// 写入 meta 键值 (覆盖) (仅测试使用)
    #[cfg(test)]
    fn meta_set(&self, key: &str, value: &str) -> Result<(), DbError> {
        self.with_tx(|tx| meta_set_on(tx, key, value))
    }

    /// 读取 unit 表全部数据 (单次查询)
    fn unit_list_all(&self) -> Result<Vec<UnitState>, DbError> {
        self.with_tx(|tx| {
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
        self.with_tx(|tx| {
            if !state.enabled && !state.masked {
                tx.execute("DELETE FROM unit WHERE name = ?1", params![state.name])
                    .map_err(|e| DbError::Sqlite(format!("unit_write: {}", e)))?;
            } else {
                tx.execute(
                    "INSERT INTO unit (name, enabled, masked) VALUES (?1, ?2, ?3)
                     ON CONFLICT(name) DO UPDATE SET enabled = excluded.enabled, masked = excluded.masked",
                    params![state.name, state.enabled as i32, state.masked as i32],
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
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute_batch("VACUUM;")
            .map_err(|e| DbError::Sqlite(format!("vacuum: {}", e)))
    }
}

impl fmt::Debug for Db {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Db").finish_non_exhaustive()
    }
}

/// 检查 `pm_sysd_w_meta` 表是否存在 (只读)
fn meta_table_exists(conn: &Connection) -> Result<bool, DbError> {
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

/// 在给定连接 (或其事务) 上读取 meta 键值
fn meta_get_on(conn: &Connection, key: &str) -> Result<Option<String>, DbError> {
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

/// 在给定连接 (或其事务) 上写入 meta 键值 (覆盖)
fn meta_set_on(conn: &Connection, key: &str, value: &str) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO pm_sysd_w_meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(|e| DbError::Sqlite(format!("meta_set: {}", e)))?;
    Ok(())
}

/// 初始化新建数据库: 建表 / 建索引 并写入初始元信息.
fn init_new_db(tx: &Transaction) -> Result<(), DbError> {
    tx.execute_batch(SCHEMA_SQL)
        .map_err(|e| DbError::Sqlite(format!("create schema: {}", e)))?;

    let now = crate::logd::now_iso();
    let version = env!("CARGO_PKG_VERSION");

    meta_set_on(tx, "program", PROGRAM_NAME)?;
    meta_set_on(tx, "schema_version", &SCHEMA_VERSION.to_string())?;
    meta_set_on(tx, "created_by_version", version)?;
    meta_set_on(tx, "last_upgrade_version", version)?;
    meta_set_on(tx, "created_at", &now)?;
    meta_set_on(tx, "updated_at", &now)?;
    Ok(())
}

/// 把数据库从 `current` 版本逐步升级到 [`SCHEMA_VERSION`].
fn upgrade(tx: &Transaction, current: u32) -> Result<(), DbError> {
    for target in (current + 1)..=SCHEMA_VERSION {
        migrate(tx, target)?;
    }

    meta_set_on(tx, "schema_version", &SCHEMA_VERSION.to_string())?;
    meta_set_on(tx, "last_upgrade_version", env!("CARGO_PKG_VERSION"))?;
    meta_set_on(tx, "updated_at", &crate::logd::now_iso())?;
    Ok(())
}
