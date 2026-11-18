//! reload 模块测试 (mock 子进程加载).

use crate::{
    conf::{self, Conf},
    db::{DbClient, UnitState},
    sysd::SysdError,
    test::TempDir,
};

use super::reload;

/// 测试替身: 直接解析配置文件, 不启动 `pm-sysd-init --reload` 进程
pub async fn run_reload_subprocess(conf_path: &str) -> Result<Conf, SysdError> {
    conf::load_conf(conf_path).map_err(|e| SysdError::Failed(e.to_string()))
}

/// 写入一条 `pm_tc = 1` 的最小配置, 返回 (临时目录, 配置路径)
fn write_conf(prefix: &str) -> (TempDir, String) {
    let dir = TempDir::new(prefix);
    let path = dir.path().join("c.toml");
    std::fs::write(&path, "pm_tc = 1\n").unwrap();
    (dir, path.to_string_lossy().into_owned())
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
async fn reload_opens_db_and_assembles_me() {
    let (_dir, conf_path) = write_conf("reload_open");
    let conf = run_reload_subprocess(&conf_path).await.unwrap();

    // 先打开并预置数据库 (reload 传 None 时会再次打开同一路径)
    let seed_db = DbClient::open(&conf.db_path).unwrap();
    seed_units(&seed_db).await;

    let (r, _db) = reload(None, &conf_path).await.unwrap();

    assert_eq!(r.me.enabled.len(), 2);
    assert_eq!(r.me.masked.len(), 2);
    assert!(r.me.enabled.contains("a.service"));
    assert!(r.me.enabled.contains("c.service"));
    assert!(r.me.masked.contains("b.service"));
    assert!(r.me.masked.contains("c.service"));
}

#[tokio::test]
async fn reload_reuses_opened_db() {
    let (_dir, conf_path) = write_conf("reload_reuse");
    let conf = run_reload_subprocess(&conf_path).await.unwrap();
    let db = DbClient::open(&conf.db_path).unwrap();
    seed_units(&db).await;

    let (r, db) = reload(Some(db), &conf_path).await.unwrap();

    assert!(r.me.enabled.contains("a.service"));
    assert!(r.me.masked.contains("b.service"));
    assert!(r.me.enabled.contains("c.service"));
    assert!(r.me.masked.contains("c.service"));

    // 返回的数据库句柄应继续可用
    let rows = db.unit_list_all().await.unwrap();
    assert_eq!(rows.len(), 3);
}

#[tokio::test]
async fn reload_bad_conf_returns_error() {
    let dir = TempDir::new("reload_bad");
    let conf_path = dir
        .path()
        .join("missing.toml")
        .to_string_lossy()
        .into_owned();

    assert!(reload(None, &conf_path).await.is_err());
}
