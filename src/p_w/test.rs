//! p_w 模块测试.

use super::*;

#[test]
fn name() {
    assert_eq!(PaW::default().name(), "windows");
}

#[test]
fn pipe_addr_format() {
    assert_eq!(pipe_addr("pm_sysd_w_m_dev"), r"\\.\pipe\pm_sysd_w_m_dev");
}
