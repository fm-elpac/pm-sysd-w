//! 数据库 schema 定义 与 迁移.

use rusqlite::Transaction;

use super::DbError;

/// 当前数据库 schema 版本 (整数, 每次 schema 变更时递增).
///
/// 在 v0.1.0 发布之前 (覆盖所有 0.1.0-* 预发布版本) 保持为 1, 不升级 schema
/// 版本.
pub const SCHEMA_VERSION: u32 = 1;

/// 本程序的数据库标识 (写入 `pm_sysd_w_meta` 表的 `program` 键, 打开时校验).
pub const PROGRAM_NAME: &str = "pm-sysd-w";

/// 单条日志 最大长度 (字节), 超出 由发送方拆分为多条.
pub const MAX_LOG_LEN: usize = 500 * 1024;

/// v1 基础表结构 (不含索引).
pub const SCHEMA_TABLES_SQL: &str = r#"
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

/// 新建数据库 时执行的 schema (表结构 + 索引).
pub const SCHEMA_SQL: &str = r#"
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
CREATE INDEX IF NOT EXISTS idx_log_unit ON log(unit, level);
CREATE INDEX IF NOT EXISTS idx_log_ts ON log(ts);
"#;

/// 执行 schema 迁移 (从 `target - 1` 升级到 `target`).
///
/// 必须在 调用方 的事务 (`tx`) 内执行, 保证 迁移原子性.
pub fn migrate(tx: &Transaction, target: u32) -> Result<(), DbError> {
    match target {
        // v1 为基础表结构, 尚无真正的迁移步骤; 仅做 幂等建表 (用于 异常的低版本号).
        1 => tx
            .execute_batch(SCHEMA_TABLES_SQL)
            .map_err(|e| DbError::Sqlite(format!("migrate to v1: {}", e))),
        other => Err(DbError::Version(format!(
            "unknown migration target {}",
            other
        ))),
    }
}
