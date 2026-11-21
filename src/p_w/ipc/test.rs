//! p_w::ipc 测试.

use super::super::acl::PipeAcl;
use super::{Listener, pipe_addr};

#[test]
fn pipe_addr_format() {
    assert_eq!(pipe_addr("pm_sysd_w_m_dev"), r"\\.\pipe\pm_sysd_w_m_dev");
}

#[tokio::test]
async fn bind_unique() {
    // 同名实例 再次绑定 失败 (first_pipe_instance 唯一性检测).
    let name = "pm_sysd_w_test_unique";
    let l1 = Listener::bind(name, PipeAcl::CurrentUser).unwrap();
    assert!(Listener::bind(name, PipeAcl::CurrentUser).is_err());
    drop(l1);
}
