//! db 模块错误.

use std::{error::Error, fmt};

/// db 模块错误.
#[derive(Debug, Clone)]
pub enum DbError {
    /// sqlite 错误
    Sqlite(String),
    /// schema 版本 不兼容 (数据库由更新版本的程序创建)
    Version(String),
    /// 数据库 `program` 标识 不是 pm-sysd-w
    Program(String),
    /// 通用错误 (携带消息)
    Msg(String),
}

impl Error for DbError {}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(m) => write!(f, "db: {}", m),
            Self::Version(m) => write!(f, "db: {}", m),
            Self::Program(m) => write!(f, "db: {}", m),
            Self::Msg(m) => write!(f, "{}", m),
        }
    }
}
