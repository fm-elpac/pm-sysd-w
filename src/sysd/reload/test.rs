//! reload 模块测试 (共享加载逻辑 assemble).

use crate::{
    conf::{self, limit::SysdLimit},
    db::{DbClient, UnitState},
    sysd::reload::assemble,
    test::TempDir,
};

/// 写入一条 `pm_tc = 1` 的最小配置, 返回 (临时目录, 解析结果)
fn write_conf(prefix: &str) -> (TempDir, conf::Conf) {
    let dir = TempDir::new(prefix);
    let path = dir.path().join("c.toml");
    std::fs::write(&path, "pm_tc = 1\n").unwrap();
    let conf = conf::load_conf(&path.to_string_lossy(), &SysdLimit::default()).unwrap();
    (dir, conf)
}

/// 预置数据库中的 mask / enable 状态
async fn seed_units(db: &DbClient) {
    let states = [
        UnitState {
            name: "a.service".to_string(),
            enabled: true,
            masked: false,
        },
        UnitState {
            name: "b.service".to_string(),
            enabled: false,
            masked: true,
        },
        UnitState {
            name: "c.service".to_string(),
            enabled: true,
            masked: true,
        },
    ];
    for s in &states {
        db.unit_write(s).await.unwrap();
    }
}

#[tokio::test]
async fn assemble_opens_db_and_assembles_me() {
    let (_dir, conf) = write_conf("reload_open");

    // 先打开并预置数据库 (assemble 传 None 时会再次打开同一路径)
    let seed_db = DbClient::open(&conf.db_path, &SysdLimit::default()).unwrap();
    seed_units(&seed_db).await;

    let (r, _db) = assemble(None, conf, &SysdLimit::default()).await.unwrap();

    assert_eq!(r.me.enabled.len(), 2);
    assert_eq!(r.me.masked.len(), 2);
    assert!(r.me.enabled.contains("a.service"));
    assert!(r.me.enabled.contains("c.service"));
    assert!(r.me.masked.contains("b.service"));
    assert!(r.me.masked.contains("c.service"));
}

#[tokio::test]
async fn assemble_reuses_opened_db() {
    let (_dir, conf) = write_conf("reload_reuse");
    let db = DbClient::open(&conf.db_path, &SysdLimit::default()).unwrap();
    seed_units(&db).await;

    let (r, db) = assemble(Some(db), conf, &SysdLimit::default())
        .await
        .unwrap();

    assert!(r.me.enabled.contains("a.service"));
    assert!(r.me.masked.contains("b.service"));
    assert!(r.me.enabled.contains("c.service"));
    assert!(r.me.masked.contains("c.service"));

    // 返回的数据库句柄应继续可用
    let rows = db.unit_list_all().await.unwrap();
    assert_eq!(rows.len(), 3);
}
