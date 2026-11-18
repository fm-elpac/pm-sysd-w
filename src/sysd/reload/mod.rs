//! init 与 unit_d 共享的加载逻辑.
//!
//! 加载配置时通过 [`crate::pa::Process`] 启动进程 `pm-sysd-init --reload`,
//! 只调用 pm-tc 读取 toml 配置, 把 [`Conf`] 序列化为 JSON 写入 stdout 后退出.
//! 本模块读取此 JSON, 再通过数据库读取 mask / enable 状态, 组装为 [`ReloadR`].
//!
//! 数据库只由 pm-sysd-init 常驻进程打开一次:
//! 首次加载传 `None`, 由本模块根据配置中的 `db_path` 打开数据库并返回.
//! daemon-reload 传 `Some(db)` 复用已打开的数据库.

use std::collections::HashSet;

use serde::Serialize;

use crate::{
    conf::Conf,
    db::DbClient,
    sysd::{SysdError, unit_d::UnitSME},
};

#[cfg(not(test))]
mod proc;

#[cfg(test)]
mod test;

#[cfg(not(test))]
use proc::run_reload_subprocess;

#[cfg(test)]
use test::run_reload_subprocess;

/// 完整加载结果
#[derive(Debug, Clone, Serialize)]
pub struct ReloadR {
    /// 配置文件解析结果
    pub conf: Conf,
    /// 数据库中的 mask / enable 状态
    pub me: UnitSME,
}

/// 加载配置与数据库状态 (供 init / daemon-reload 使用).
///
/// `db` 为 `None` 时打开数据库并返回.
/// 为 `Some` 时复用该数据库, 返回的 `DbClient` 可忽略.
pub async fn reload(
    db: Option<DbClient>,
    conf_path: &str,
) -> Result<(ReloadR, DbClient), SysdError> {
    let conf = run_reload_subprocess(conf_path).await?;
    let db = match db {
        Some(db) => db,
        None => DbClient::open(&conf.db_path).map_err(|e| SysdError::Failed(e.to_string()))?,
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
