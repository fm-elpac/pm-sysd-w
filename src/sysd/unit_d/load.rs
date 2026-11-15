//! UnitD 的加载 / 持久化辅助 (初始化与 daemon-reload 相关).
//!
//! - 重新读取配置文件: 使用专用同步线程 (pm-tc 为同步实现).
//! - 从数据库加载全部 enable / mask 列表.
//! - 向数据库写入单个 enable / mask 更新.
//! - daemon-reload 的后台 task: 完成后回传内部事件给 actor.

use std::thread;

use tokio::sync::{mpsc, oneshot};

use crate::{
    conf::{Conf, load_conf},
    db::{DbClient, DbError},
    jsonl::SdRes,
    sysd::SysdError,
};

use super::{actor::M, sm::WriteUnit, unit_s::UnitSME};

use super::super::send_reply;

/// 转换 db 错误
fn db_err(e: DbError) -> SysdError {
    SysdError::Failed(e.to_string())
}

/// 重新读取配置文件 (专用同步线程)
pub async fn load_conf_sync(conf_path: &str) -> Result<Conf, SysdError> {
    let conf_path = conf_path.to_string();
    let (tx, rx) = oneshot::channel();
    // pm-tc 为同步实现, 放到专用同步线程中读取, 不阻塞 tokio 异步线程
    thread::spawn(move || {
        let r = load_conf(&conf_path).map_err(|e| e.to_string());
        send_reply(tx, r);
    });

    let loaded = rx
        .await
        .map_err(|_| SysdError::Failed("reload thread stopped".to_string()))?;
    loaded.map_err(SysdError::Failed)
}

/// 从数据库加载全部 mask / enable 的 unit 名称
pub async fn load_masked_enabled(db: &DbClient) -> Result<UnitSME, SysdError> {
    let masked = db.unit_list_masked().await.map_err(db_err)?;
    let enabled = db.unit_list_enabled().await.map_err(db_err)?;
    Ok(UnitSME {
        masked: masked.into_iter().collect(),
        enabled: enabled.into_iter().collect(),
    })
}

/// 向数据库写入单个  mask / enable 更新
pub async fn write_unit(db: &DbClient, w: &WriteUnit) -> Result<(), SysdError> {
    if let Some(masked) = w.masked {
        db.unit_set_masked(&w.name, masked).await.map_err(db_err)?;
    }
    if let Some(enabled) = w.enabled {
        db.unit_set_enabled(&w.name, enabled)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}

/// 触发 daemon-reload (spawn 独立 task, 主循环不等待).
///
/// 依次完成: (1) 重新读取配置文件; (2) 从数据库重新读取 mask/enable 状态.
/// 成功后回传 [`M::ReloadOk`] (由状态机更新状态并回复); 任一步失败则回传
/// [`M::ReloadErr`] (由状态机清理 reload 状态并回复).
pub fn spawn_reload(
    db: DbClient,
    conf_path: String,
    tx: mpsc::Sender<M>,
    reply: oneshot::Sender<SdRes>,
) {
    tokio::spawn(async move {
        let conf = match load_conf_sync(&conf_path).await {
            Ok(conf) => conf,
            Err(e) => {
                send_reload_err(&tx, e.to_string(), reply).await;
                return;
            }
        };

        let me = match load_masked_enabled(&db).await {
            Ok(me) => me,
            Err(e) => {
                send_reload_err(&tx, e.to_string(), reply).await;
                return;
            }
        };

        if tx.send(M::ReloadOk(conf, me, reply)).await.is_err() {
            log::error!("reload: UnitD stopped");
        }
    });
}

/// 向 actor 回传 reload 失败 (内部事件)
async fn send_reload_err(tx: &mpsc::Sender<M>, e: String, reply: oneshot::Sender<SdRes>) {
    if tx.send(M::ReloadErr(e, reply)).await.is_err() {
        log::error!("reload: UnitD stopped");
    }
}
