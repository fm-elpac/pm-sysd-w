//! p_w::acl 测试.

use super::super::ipc::pipe_addr;
use super::{PipeAcl, create_server};

#[tokio::test]
async fn acl_build_all_variants() {
    // 构建 3 种 ACL 的安全描述符 应全部成功 (在 Windows 上运行).
    for (i, pipe_acl) in [
        PipeAcl::System,
        PipeAcl::SystemReadOnly,
        PipeAcl::CurrentUser,
    ]
    .into_iter()
    .enumerate()
    {
        let name = format!("pm_sysd_w_test_acl_{}", i);
        let _server = create_server(&pipe_addr(&name), true, pipe_acl).unwrap();
    }
}
