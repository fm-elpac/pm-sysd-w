//! SysD 状态机 (仅计算/决策, 不执行操作).

use crate::jsonl::SdReqSysd;

use super::super::init::Shutdown;

/// SysD 自己处理 `SdReq::SysD` 时的决策结果.
pub struct SmAction {
    /// 日志级别.
    pub log_level: &'static str,
    /// 日志内容.
    pub log_msg: &'static str,
    /// 退出方式 (通知 sysd::init).
    pub shutdown: Shutdown,
}

/// SysD 状态机 (当前无内部状态, 只处理自己负责的请求).
pub struct SysDSM;

impl SysDSM {
    /// 处理 SysD 自己负责的请求, 返回决策.
    pub fn handle(&self, req: &SdReqSysd) -> SmAction {
        match req {
            SdReqSysd::SoftReboot => SmAction {
                log_level: "info",
                log_msg: "soft-reboot requested",
                shutdown: Shutdown::SoftReboot,
            },
            SdReqSysd::Exit => SmAction {
                log_level: "info",
                log_msg: "exit requested",
                shutdown: Shutdown::Exit,
            },
        }
    }
}
