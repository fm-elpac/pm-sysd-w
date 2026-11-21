//! 数据库连接与底层辅助函数 (`Db`).

#[cfg(test)]
mod test;

use std::{
    fmt, fs,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use rusqlite::{Connection, Transaction, params};

use super::{
    error::DbError,
    schema::{PROGRAM_NAME, SCHEMA_SQL, SCHEMA_TABLES_SQL, SCHEMA_VERSION, migrate},
};

/// 数据库连接 (封装 sqlite), 可 clone (共享同一连接)
#[derive(Clone)]
pub(super) struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// 打开 (或创建) 数据库, 日志表创建索引 (默认).
    ///
    /// 版本检查, 初始化, 升级, 全部封装在 **单个事务** 中 (见 [`Self::init_or_upgrade`]).
    pub(super) fn open(path: &str) -> Result<Self, DbError> {
        Self::open_impl(path, false)
    }

    /// 打开 (或创建) 数据库, 日志表不创建索引 (写入优化模式).
    ///
    /// 仅影响新建数据库的 schema, 已有数据库保持不变.
    pub(super) fn open_log_no_index(path: &str) -> Result<Self, DbError> {
        Self::open_impl(path, true)
    }

    /// 打开 (或创建) 数据库的内部实现, `log_no_index` 控制日志表是否创建索引.
    fn open_impl(path: &str, log_no_index: bool) -> Result<Self, DbError> {
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
            conn: Arc::new(Mutex::new(conn)),
        };

        db.init_or_upgrade(path, log_no_index)?;
        Ok(db)
    }

    /// 锁数据库连接一次并返回连接守卫 (供无事务的底层操作使用)
    pub(super) fn lock_conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 在单个事务中完成: 版本检查, 初始化, 升级.
    ///
    /// - 仅当 `pm_sysd_w_meta` 表不存在时才创建 schema (新数据库).
    ///   建表 / 建索引 / 初始元信息, 都在该事务中.
    /// - 表已存在时先读取 `program` / `schema_version` (只读).
    /// - 仅当需要升级 (旧版本) 时才执行迁移.
    fn init_or_upgrade(&self, path: &str, log_no_index: bool) -> Result<(), DbError> {
        self.with_tx(|tx| {
            if !meta_table_exists(tx)? {
                init_new_db(tx, log_no_index)?;
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
    pub(super) fn with_tx<T>(
        &self,
        f: impl FnOnce(&Transaction) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        let mut conn = self.lock_conn();
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

/// 初始化新建数据库: 建表 / 建索引 并写入初始元信息
///
/// `log_no_index` 为 `true` 时日志表不创建索引 (写入优化模式).
fn init_new_db(tx: &Transaction, log_no_index: bool) -> Result<(), DbError> {
    let schema_sql = if log_no_index {
        SCHEMA_TABLES_SQL
    } else {
        SCHEMA_SQL
    };
    tx.execute_batch(schema_sql)
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

/// 把数据库从 `current` 版本逐步升级到 [`SCHEMA_VERSION`]
fn upgrade(tx: &Transaction, current: u32) -> Result<(), DbError> {
    for target in (current + 1)..=SCHEMA_VERSION {
        migrate(tx, target)?;
    }

    meta_set_on(tx, "schema_version", &SCHEMA_VERSION.to_string())?;
    meta_set_on(tx, "last_upgrade_version", env!("CARGO_PKG_VERSION"))?;
    meta_set_on(tx, "updated_at", &crate::logd::now_iso())?;
    Ok(())
}
