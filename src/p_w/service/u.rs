//! p_w::service 的 unsafe 实现 (Win32 SCM API 边界).
//!
//! 本模块集中所有 unsafe 的 SCM 调用与 FFI 回调. 能 safe 的代码不放进来.
#![allow(unsafe_code)]

use std::sync::{
    OnceLock,
    atomic::{AtomicU32, Ordering},
};

use windows::{
    Win32::{
        Foundation::ERROR_FAILED_SERVICE_CONTROLLER_CONNECT,
        System::Services::{
            RegisterServiceCtrlHandlerW, SERVICE_ACCEPT_SHUTDOWN, SERVICE_ACCEPT_STOP,
            SERVICE_CONTROL_INTERROGATE, SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP,
            SERVICE_RUNNING, SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STATUS_CURRENT_STATE,
            SERVICE_STATUS_HANDLE, SERVICE_STOP_PENDING, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW,
            SERVICE_USER_OWN_PROCESS, SetServiceStatus, StartServiceCtrlDispatcherW,
        },
    },
    core::{HRESULT, PCWSTR, PWSTR},
};

use super::{PwError, stop_event, to_wide};

/// SCM 登记用的服务名 (UTF-16, 以 NUL 结尾).
///
/// own-process 服务下 SCM 不校验此名称, 但仍要求非空.
static SERVICE_NAME: OnceLock<Vec<u16>> = OnceLock::new();

/// 服务状态句柄 (非 Sync, 只能使用 `static mut`)
static mut STATUS_HANDLE: SERVICE_STATUS_HANDLE = SERVICE_STATUS_HANDLE(std::ptr::null_mut());

/// 最近一次上报的服务状态码 (`SERVICE_STATUS_CURRENT_STATE` 的 `.0`)
static CURRENT_STATE: AtomicU32 = AtomicU32::new(SERVICE_START_PENDING.0);

/// 执行 SCM 服务分发 (由 safe 层的 `run_dispatcher` 调用).
///
/// 返回 `Ok(true)` 表示已作为服务运行并结束; `Ok(false)` 表示非服务模式.
pub(super) fn dispatch(service_name: &str) -> Result<bool, PwError> {
    if SERVICE_NAME.set(to_wide(service_name)).is_err() {
        return Err(PwError::Msg(
            "service dispatcher already initialized".into(),
        ));
    }

    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR(service_name_ptr() as *mut u16),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: PWSTR(std::ptr::null_mut()),
            lpServiceProc: None,
        },
    ];

    match unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } {
        Ok(()) => Ok(true),
        Err(e) if e.code() == HRESULT::from_win32(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT.0) => {
            Ok(false)
        }
        Err(e) => Err(PwError::Msg(format!("StartServiceCtrlDispatcherW: {}", e))),
    }
}

/// SCM 服务主函数 (在 SCM 新建的线程中执行)
unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    super::IS_SERVICE.store(true, Ordering::SeqCst);

    let handle = match unsafe {
        RegisterServiceCtrlHandlerW(PCWSTR(service_name_ptr()), Some(control_handler))
    } {
        Ok(h) => h,
        Err(e) => {
            log::error!("RegisterServiceCtrlHandlerW: {}", e);
            return;
        }
    };
    unsafe { STATUS_HANDLE = handle };

    // 启动中: 此时不接受控制请求 (dwControlsAccepted = 0)
    report_state(SERVICE_START_PENDING, 0, 0);
    // 初始化完成即进入运行状态 (body 内部为主循环, 运行期间一直阻塞)
    report_state(
        SERVICE_RUNNING,
        SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN,
        0,
    );

    let body = super::BODY
        .get()
        .copied()
        .expect("service body must be initialized");
    match body(stop_event()) {
        Ok(()) => report_state(SERVICE_STOPPED, 0, 0),
        Err(msg) => {
            log::error!("service body failed: {}", msg);
            // 非零 Win32 退出码表示服务启动/运行失败
            report_state(SERVICE_STOPPED, 0, 1);
        }
    }
}

/// 服务控制处理回调
unsafe extern "system" fn control_handler(control: u32) {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            stop_event().request();
            report_state(SERVICE_STOP_PENDING, 0, 0);
        }
        SERVICE_CONTROL_INTERROGATE => {
            // 查询: 上报当前状态. 运行中才接受停止/关机控制
            let state = current_state();
            let controls = if state == SERVICE_RUNNING {
                SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
            } else {
                0
            };
            report_state(state, controls, 0);
        }
        _ => {}
    }
}

/// 向 SCM 上报服务状态, 并记录当前状态.
///
/// 上报失败无恢复手段 (且此模块允许静默忽略错误),
/// 因此忽略 `SetServiceStatus` 的返回值.
fn report_state(state: SERVICE_STATUS_CURRENT_STATE, controls: u32, exit_code: u32) {
    let handle = unsafe { STATUS_HANDLE };
    if handle.is_invalid() {
        return;
    }

    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_USER_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: controls,
        dwWin32ExitCode: exit_code,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 0,
    };
    // 见函数注释: 上报失败不可恢复, 忽略
    unsafe {
        let _ = SetServiceStatus(handle, &status);
    }

    CURRENT_STATE.store(state.0, Ordering::SeqCst);
}

/// 读取最近一次上报的服务状态
fn current_state() -> SERVICE_STATUS_CURRENT_STATE {
    SERVICE_STATUS_CURRENT_STATE(CURRENT_STATE.load(Ordering::SeqCst))
}

/// 返回 SCM 登记服务名的 UTF-16 缓冲区首地址
fn service_name_ptr() -> *const u16 {
    SERVICE_NAME
        .get()
        .expect("service name initialized")
        .as_ptr()
}
