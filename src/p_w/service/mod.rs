//! p_w::service: Windows 服务 (SCM) 集成.
//!
//! 让 pm-sysd-init 能够作为 Windows 服务运行:
//!
//! - 由 SCM 启动时调用 `StartServiceCtrlDispatcherW`, 进入服务分发.
//!
//! - 在服务主函数中注册控制处理回调, 并向 SCM 上报状态.
//!
//! - 收到 `SERVICE_CONTROL_STOP` / `SERVICE_CONTROL_SHUTDOWN`
//!   时置位全局停止事件, 供上层 (`pa::spawn_signal_watch`) 触发优雅退出.
//!
//! 本模块保持 safe, unsafe 的 Win32 SCM 调用集中在 [`u`] 模块.

use std::{
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use tokio::sync::Notify;

use super::PwError;

mod u;

/// 服务主体回调: 在 SCM 服务线程中被调用, 完成实际工作.
///
/// 参数为全局停止请求事件. 返回 `Err` 表示服务启动失败.
pub type ServiceBody = fn(&'static StopEvent) -> Result<(), String>;

/// 服务主体回调 (由 [`run_dispatcher`] 设置)
static BODY: OnceLock<ServiceBody> = OnceLock::new();

/// 全局停止请求事件
static STOP: OnceLock<StopEvent> = OnceLock::new();

/// 当前进程是否运行于服务模式 (由 SCM 启动)
static IS_SERVICE: AtomicBool = AtomicBool::new(false);

/// 服务停止请求事件 (跨线程共享)
pub struct StopEvent {
    notify: Notify,
    flag: AtomicBool,
}

impl StopEvent {
    fn new() -> Self {
        Self {
            notify: Notify::new(),
            flag: AtomicBool::new(false),
        }
    }

    /// 置位停止请求并唤醒所有等待者
    fn request(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// 是否已请求停止
    pub fn is_requested(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// 等待停止请求
    pub async fn wait(&self) {
        if self.is_requested() {
            return;
        }
        loop {
            self.notify.notified().await;
            if self.is_requested() {
                return;
            }
        }
    }
}

/// 返回全局停止请求事件
pub fn stop_event() -> &'static StopEvent {
    STOP.get_or_init(StopEvent::new)
}

/// 当前进程是否运行于服务模式
pub fn is_service() -> bool {
    IS_SERVICE.load(Ordering::SeqCst)
}

/// 尝试作为 Windows 服务运行.
///
/// 由 pm-sysd-init 入口在解析命令参数前调用. `body` 在服务线程中被调用.
///
/// 返回:
///
/// - `Ok(true)`: 当前进程由 SCM 启动, 且服务已正常结束.
/// - `Ok(false)`: 当前进程不是由 SCM 启动 (普通命令行), 未进入服务模式.
/// - `Err(e)`: 分发失败.
pub fn run_dispatcher(service_name: &str, body: ServiceBody) -> Result<bool, PwError> {
    if BODY.set(body).is_err() {
        return Err(PwError::Msg(
            "service dispatcher already initialized".into(),
        ));
    }
    u::dispatch(service_name)
}

/// 字符串转以 NUL 结尾的 UTF-16 缓冲区 (供 unsafe 层使用)
fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(test)]
mod test;
