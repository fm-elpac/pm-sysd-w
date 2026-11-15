//! sysd 模块公共辅助函数.

use tokio::sync::oneshot;

/// 通过 oneshot 发送回执. 若接收方已退出 (发送失败), 记录 error 日志并丢弃回执.
///
/// 所有通过 oneshot 向调用方回复的场景都应使用本函数, 避免调用方提前退出导致 panic.
pub fn send_reply<T>(tx: oneshot::Sender<T>, value: T) {
    if tx.send(value).is_err() {
        log::error!("send oneshot reply failed (receiver dropped)");
    }
}
