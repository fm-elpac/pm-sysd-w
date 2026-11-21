//! init 与 unit_d 共享的加载逻辑.
//!
//! 配置加载通过 [`Reloader`] trait 抽象: 生产实现 [`proc::SelfReloader`]
//! 启动 `pm-sysd-init --reload` 子进程, 只调用 pm-tc 读取 toml 配置,
//! 把 [`Conf`] 序列化为 JSON 写入 stdout 后退出. 本模块读取此 JSON,
//! 再通过数据库读取 mask / enable 状态, 组装为 [`ReloadR`].
//!
//! 数据库只由 pm-sysd-init 常驻进程打开一次:
//! 首次加载传 `None`, 由 [`assemble`] 根据配置中的 `db_path` 打开数据库并返回.
//! daemon-reload 传 `Some(db)` 复用已打开的数据库.

use std::{collections::HashSet, future::Future, pin::Pin};

use serde::Serialize;

use crate::{
    conf::{Conf, limit::SysdLimit},
    db::DbClient,
    sysd::{SysdError, unit_d::UnitSME},
};

pub mod proc;
pub mod sub;

#[cfg(test)]
mod test;

/// 完整加载结果
#[derive(Debug, Clone, Serialize)]
pub struct ReloadR {
    /// 配置文件解析结果
    pub conf: Conf,
    /// 数据库中的 mask / enable 状态
    pub me: UnitSME,
}

/// 加载器抽象: 读取配置与数据库状态, 返回完整加载结果.
///
/// `async fn` 在 trait 中不是 dyn-compatible, 因此这里返回 boxed future,
/// 以支持 `Box<dyn Reloader>`.
pub trait Reloader: Send + Sync {
    fn reload<'a>(
        &'a self,
        db: Option<DbClient>,
        conf_path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(ReloadR, DbClient), SysdError>> + Send + 'a>>;
}

/// 读取数据库中的 mask / enable 状态, 组装为 [`ReloadR`].
///
/// `db` 为 `None` 时打开数据库并返回.
/// 为 `Some` 时复用该数据库, 返回的 `DbClient` 可忽略.
pub async fn assemble(
    db: Option<DbClient>,
    conf: Conf,
    limit: &SysdLimit,
) -> Result<(ReloadR, DbClient), SysdError> {
    let db = match db {
        Some(db) => db,
        None => {
            let open_res = if conf.db_log_no_index {
                DbClient::open_log_no_index(&conf.db_path, limit)
            } else {
                DbClient::open(&conf.db_path, limit)
            };
            open_res.map_err(|e| SysdError::Failed(e.to_string()))?
        }
    };

    let rows = db
        .unit_list_all()
        .await
        .map_err(|e| SysdError::Failed(e.to_string()))?;

    let mut masked = HashSet::new();
    let mut enabled = HashSet::new();
    for u in rows {
        if u.masked {
            masked.insert(u.name.clone());
        }
        if u.enabled {
            enabled.insert(u.name);
        }
    }

    let r = ReloadR {
        conf,
        me: UnitSME { masked, enabled },
    };
    Ok((r, db))
}
