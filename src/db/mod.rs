//! 数据库 (sqlite) 相关功能的抽象/封装.
//!
//! pm-sysd-init 使用 sqlite 数据库存储持久状态 (例如 unit 是否 enable/mask), 以及
//! 日志数据. 数据库格式属于内部 API, 不作为外部稳定 API 提供.
//!
//! 数据库 schema 与版本/升级规则见 `doc/dev/db.md`.

mod error;
#[cfg(test)]
mod test;

pub use error::DbError;

use std::{fmt, fs, path::Path, sync::Mutex};

use chrono::SecondsFormat;
use rusqlite::{Connection, params};

/// 当前数据库 schema 版本 (整数, 每次 schema 变更时递增).
pub const SCHEMA_VERSION: u32 = 1;

/// 本程序的数据库标识 (写入 `pm_sysd_w_meta` 表的 `program` 键, 打开时校验).
pub const PROGRAM_NAME: &str = "pm-sysd-w";

/// 创建 schema (v1) 的 SQL.
const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS pm_sysd_w_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS unit (
  name    TEXT PRIMARY KEY,
  enabled INTEGER NOT NULL DEFAULT 0,
  masked  INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS log (
  id    INTEGER PRIMARY KEY AUTOINCREMENT,
  ts    TEXT NOT NULL,
  level TEXT NOT NULL,
  unit  TEXT,
  msg   TEXT NOT NULL
);
"#;

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

/// 数据库连接 (封装 sqlite).
pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    /// 打开 (或创建) 数据库, 检查 schema 版本, 必要时自动升级.
    pub fn open(path: &str) -> Result<Self, DbError> {
        // 确保 上级目录 存在.
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)
                    .map_err(|e| DbError::Sqlite(format!("create dir `{:?}`: {}", parent, e)))?;
            }
        }

        let conn = Connection::open(path)
            .map_err(|e| DbError::Sqlite(format!("open `{}`: {}", path, e)))?;

        // 创建 schema (v1 的表结构).
        conn.execute_batch(SCHEMA_SQL)
            .map_err(|e| DbError::Sqlite(format!("create schema: {}", e)))?;

        let db = Self {
            conn: Mutex::new(conn),
        };

        // 检查 / 初始化 meta.
        match db.meta_get("program")? {
            None => db.init_new_db()?,
            Some(p) if p != PROGRAM_NAME => {
                return Err(DbError::Program(format!(
                    "database `{}` belongs to `{}`, not `{}`",
                    path, p, PROGRAM_NAME
                )));
            }
            Some(_) => db.check_upgrade()?,
        }

        Ok(db)
    }

    /// 初始化 新建数据库 的 meta.
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

    /// 检查 schema 版本, 必要时自动升级.
    fn check_upgrade(&self) -> Result<(), DbError> {
        let current = self
            .meta_get("schema_version")?
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);

        if current > SCHEMA_VERSION {
            return Err(DbError::Version(format!(
                "database schema version `{}` is newer than supported `{}`",
                current, SCHEMA_VERSION
            )));
        }

        for target in (current + 1)..=SCHEMA_VERSION {
            migrate(self, target)?;
        }

        // 更新 meta (schema 版本 / 升级版本 / 时间).
        self.meta_set("schema_version", &SCHEMA_VERSION.to_string())?;
        self.meta_set("last_upgrade_version", env!("CARGO_PKG_VERSION"))?;
        self.meta_set("updated_at", &now_iso())?;
        Ok(())
    }

    /// 读取 meta 键值.
    pub fn meta_get(&self, key: &str) -> Result<Option<String>, DbError> {
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

    /// 写入 meta 键值 (覆盖).
    pub fn meta_set(&self, key: &str, value: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO pm_sysd_w_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map_err(|e| DbError::Sqlite(format!("meta_set: {}", e)))?;
        Ok(())
    }

    /// 确保 unit 在 `unit` 表中存在 (不存在则插入默认行).
    pub fn unit_ensure(&self, name: &str) -> Result<(), DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO unit (name, enabled, masked) VALUES (?1, 0, 0)
             ON CONFLICT(name) DO NOTHING",
            params![name],
        )
        .map_err(|e| DbError::Sqlite(format!("unit_ensure: {}", e)))?;
        Ok(())
    }

    /// 设置 unit 的 enable 状态.
    pub fn unit_set_enabled(&self, name: &str, enabled: bool) -> Result<(), DbError> {
        self.unit_ensure(name)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE unit SET enabled = ?1 WHERE name = ?2",
            params![enabled as i32, name],
        )
        .map_err(|e| DbError::Sqlite(format!("unit_set_enabled: {}", e)))?;
        Ok(())
    }

    /// 读取 unit 的 enable 状态 (不存在则 false).
    pub fn unit_get_enabled(&self, name: &str) -> Result<bool, DbError> {
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

    /// 设置 unit 的 mask 状态.
    pub fn unit_set_masked(&self, name: &str, masked: bool) -> Result<(), DbError> {
        self.unit_ensure(name)?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE unit SET masked = ?1 WHERE name = ?2",
            params![masked as i32, name],
        )
        .map_err(|e| DbError::Sqlite(format!("unit_set_masked: {}", e)))?;
        Ok(())
    }

    /// 读取 unit 的 mask 状态 (不存在则 false).
    pub fn unit_get_masked(&self, name: &str) -> Result<bool, DbError> {
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

    /// 追加一条日志.
    pub fn log_append(
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

    /// 读取 最近的 若干条日志 (可选 按 unit 过滤).
    pub fn log_recent(&self, unit: Option<&str>, limit: u32) -> Result<Vec<LogEntry>, DbError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let sql = "SELECT id, ts, level, unit, msg FROM log
                   WHERE (?1 IS NULL OR unit = ?1) ORDER BY id DESC LIMIT ?2";

        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| DbError::Sqlite(format!("log_recent: {}", e)))?;

        let rows = stmt
            .query_map(params![unit, limit], |row| {
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
}

impl fmt::Debug for Db {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Db").finish_non_exhaustive()
    }
}

/// 执行 schema 迁移 (从 `target - 1` 升级到 `target`).
///
/// 当前 `SCHEMA_VERSION = 1`, 尚无迁移步骤. 后续每次 schema 变更需补充对应迁移代码.
fn migrate(_db: &Db, _target: u32) -> Result<(), DbError> {
    Ok(())
}

/// 当前时间 (UTC, ISO 8601, 毫秒精度).
fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}
