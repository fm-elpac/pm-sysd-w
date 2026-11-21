//! db 模块测试 (DbRw: 读写线程相关操作).

use crate::test::TempDir;

use super::super::{UnitState, util::Db};
use super::DbRw;

fn tmp_db(dir: &TempDir, name: &str) -> String {
    dir.path().join(name).to_string_lossy().into_owned()
}

fn open_rw(dir: &TempDir, name: &str) -> DbRw {
    let db = Db::open(&tmp_db(dir, name)).unwrap();
    DbRw::new(db)
}

#[test]
fn open_and_persist() {
    let dir = TempDir::new("db");
    let rw = open_rw(&dir, "a.db");

    let enabled = UnitState {
        name: "a.service".into(),
        enabled: true,
        masked: false,
    };
    rw.unit_write(&enabled).unwrap();
    assert_eq!(rw.unit_list_all().unwrap(), vec![enabled]);

    let masked = UnitState {
        name: "a.service".into(),
        enabled: true,
        masked: true,
    };
    rw.unit_write(&masked).unwrap();
    assert_eq!(rw.unit_list_all().unwrap(), vec![masked]);
}

#[test]
fn reopen_keeps_state() {
    let dir = TempDir::new("db");

    let state = UnitState {
        name: "a.service".into(),
        enabled: true,
        masked: false,
    };
    {
        let rw = open_rw(&dir, "a.db");
        rw.unit_write(&state).unwrap();
    }
    {
        let rw = open_rw(&dir, "a.db");
        assert_eq!(rw.unit_list_all().unwrap(), vec![state]);
    }
}

#[test]
fn unit_write_default_deletes_row() {
    let dir = TempDir::new("db");
    let rw = open_rw(&dir, "a.db");

    // 全默认值: 不保留行.
    let default = UnitState {
        name: "a.service".into(),
        enabled: false,
        masked: false,
    };
    rw.unit_write(&default).unwrap();
    assert!(rw.unit_list_all().unwrap().is_empty());

    // 先写非默认, 再写默认, 行被删除 (自动清理).
    rw.unit_write(&UnitState {
        name: "a.service".into(),
        enabled: true,
        masked: false,
    })
    .unwrap();
    assert_eq!(rw.unit_list_all().unwrap().len(), 1);

    rw.unit_write(&default).unwrap();
    assert!(rw.unit_list_all().unwrap().is_empty());
}

#[test]
fn vacuum_ok() {
    let dir = TempDir::new("db");
    let rw = open_rw(&dir, "a.db");

    rw.unit_write(&UnitState {
        name: "a.service".into(),
        enabled: true,
        masked: false,
    })
    .unwrap();
    rw.vacuum().unwrap();
}
