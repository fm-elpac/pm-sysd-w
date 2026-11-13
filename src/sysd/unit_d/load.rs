//! UnitD 的加载 / 持久化辅助 (初始化与 daemon-reload 相关).
//!
//! - 重新读取配置文件: 使用专用 同步线程 (pm-tc 为同步实现).
//! - 从数据库加载全部 enable / mask 列表.
//! - 向数据库写入单个 enable / mask 更新.

use std::thread;

use tokio::sync::{mpsc, oneshot};

use crate::{
    conf::{Conf, load_conf},
    db::{DbClient, DbError},
    jsonl::SdRes,
};

use super::{SysdError, UnitDM, actor::WriteUnit, unit_s::UnitSME};

/// 转换 db 错误
fn db_err(e: DbError) -> SysdError {
    SysdError::Failed(e.to_string())
}

/// 重新读取配置文件 (专用同步线程)
async fn load_conf_sync(conf_path: &str) -> Result<Conf, SysdError> {
    let conf_path = conf_path.to_string();
    let (tx, rx) = oneshot::channel();
    // pm-tc 为同步实现, 放到专用同步线程中读取, 不阻塞 tokio 异步线程
    thread::spawn(move || {
        let r = load_conf(&conf_path).map_err(|e| e.to_string());
        tx.send(r).expect("reload reply channel closed");
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
/// 加载成功后回传 [`UnitDM::ReloadOk`] (由 UnitD 更新状态并回复).
/// 失败时直接回复调用方.
pub fn reload(conf_path: String, reply: oneshot::Sender<SdRes>, ud: mpsc::Sender<UnitDM>) {
    tokio::spawn(async move {
        match load_conf_sync(&conf_path).await {
            Ok(conf) => {
                ud.send(UnitDM::ReloadOk(conf, reply)).await.unwrap();
            }
            Err(e) => {
                reply.send(SdRes::Err(e.to_string())).unwrap();
            }
        }
    });
}
