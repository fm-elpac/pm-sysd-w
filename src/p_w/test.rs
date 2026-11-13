//! p_w 模块测试.

use super::{Listener, acl, pipe_addr};

#[test]
fn pipe_addr_format() {
    assert_eq!(pipe_addr("pm_sysd_w_m_dev"), r"\\.\pipe\pm_sysd_w_m_dev");
}

#[tokio::test]
async fn acl_build_all_variants() {
    // 构建 3 种 ACL 的安全描述符 应全部成功 (在 Windows 上运行).
    for (i, pipe_acl) in [
        acl::PipeAcl::System,
        acl::PipeAcl::SystemReadOnly,
        acl::PipeAcl::CurrentUser,
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("pm_sysd_w_test_acl_{}", i);
        let _server = acl::create_server(&pipe_addr(&name), true, pipe_acl).unwrap();
    }
}

#[tokio::test]
async fn bind_unique() {
    // 同名实例 再次绑定 失败 (first_pipe_instance 唯一性检测).
    let name = "pm_sysd_w_test_unique";
    let l1 = Listener::bind(name, acl::PipeAcl::CurrentUser).unwrap();
    assert!(Listener::bind(name, acl::PipeAcl::CurrentUser).is_err());
    drop(l1);
}
