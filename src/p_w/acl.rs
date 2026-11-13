//! Windows 命名管道 ACL 辅助 (安全描述符).
//!
//! 用于给 pm-sysd-init 创建的命名管道设置 DACL, 防止越权访问:
//!
//! - [`PipeAcl::System`]: 仅管理员 / SYSTEM.
//! - [`PipeAcl::SystemReadOnly`]: 本机普通用户 可连接 (读+写), 只读限制由 init
//!   在命令层执行.
//! - [`PipeAcl::CurrentUser`]: 仅当前用户.
//!
//! Windows 安全描述符 / ACL API 均为 `unsafe fn`, 本模块 是 p_w 中 允许
//! `unsafe` 的两个模块之一 (另一个是 `console`): 以 `#[allow(unsafe_code)]`
//! 覆盖 crate 级 `deny(unsafe_code)`, 并把 unsafe 调用集中在此处, 对外提供
//! 安全接口.
#![allow(unsafe_code)]

use std::{
    ffi::{OsStr, c_void},
    io, iter,
    mem::size_of,
    os::windows::ffi::OsStrExt,
    slice,
};

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, HANDLE},
        Security::{
            ACL, ACL_REVISION, AddAccessAllowedAce, CreateWellKnownSid, GetLengthSid,
            GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor, PSECURITY_DESCRIPTOR,
            PSID, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SetSecurityDescriptorDacl, TOKEN_QUERY,
            TOKEN_USER, TokenUser, WELL_KNOWN_SID_TYPE, WinAuthenticatedUserSid,
            WinBuiltinAdministratorsSid, WinLocalSystemSid,
        },
        Storage::FileSystem::{FILE_ALL_ACCESS, FILE_GENERIC_READ, FILE_GENERIC_WRITE},
        System::{
            Pipes::WaitNamedPipeW,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    },
    core::{BOOL, PCWSTR},
};

use super::PwError;

/// 安全描述符 修订号 (固定).
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

/// 命名管道 权限级别.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeAcl {
    /// 系统级主管道: 仅管理员 / SYSTEM.
    System,
    /// 系统级只读管道: 本机普通用户 可连接 (读+写), 只读限制由 init 命令层执行.
    SystemReadOnly,
    /// 用户级 / 自定义实例: 仅当前用户.
    CurrentUser,
}

/// 保持 安全描述符 / ACL 缓冲区 存活, 并提供 传递给 `CreateNamedPipeW` 的指针.
struct SecurityAttributes {
    sa: SECURITY_ATTRIBUTES,
    _sd: Box<SECURITY_DESCRIPTOR>,
    _acl: Vec<u8>,
}

impl SecurityAttributes {
    fn new(kind: PipeAcl) -> Result<Self, PwError> {
        let (sids, masks) = match kind {
            PipeAcl::System => (
                vec![
                    well_known_sid(WinLocalSystemSid)?,
                    well_known_sid(WinBuiltinAdministratorsSid)?,
                ],
                vec![FILE_ALL_ACCESS.0, FILE_ALL_ACCESS.0],
            ),
            PipeAcl::SystemReadOnly => (
                vec![well_known_sid(WinAuthenticatedUserSid)?],
                // 允许 普通用户 读+写 以完成 请求/响应 通信; 只读 由 init 命令层 限制.
                vec![FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0],
            ),
            PipeAcl::CurrentUser => (vec![current_user_sid()?], vec![FILE_ALL_ACCESS.0]),
        };

        // ACL 缓冲区 (足够容纳 少量 ACE).
        let mut acl_buf = vec![0u8; 1024];
        let acl = acl_buf.as_mut_ptr() as *mut ACL;

        unsafe { InitializeAcl(acl, acl_buf.len() as u32, ACL_REVISION) }
            .map_err(|e| PwError::Ipc(format!("InitializeAcl: {}", e)))?;

        for (sid, mask) in sids.iter().zip(masks.iter()) {
            unsafe {
                AddAccessAllowedAce(acl, ACL_REVISION, *mask, PSID(sid.as_ptr() as *mut c_void))
            }
            .map_err(|e| PwError::Ipc(format!("AddAccessAllowedAce: {}", e)))?;
        }

        let mut sd = Box::new(SECURITY_DESCRIPTOR::default());
        let sd_ptr = PSECURITY_DESCRIPTOR(&mut *sd as *mut SECURITY_DESCRIPTOR as *mut c_void);
        unsafe { InitializeSecurityDescriptor(sd_ptr, SECURITY_DESCRIPTOR_REVISION) }
            .map_err(|e| PwError::Ipc(format!("InitializeSecurityDescriptor: {}", e)))?;
        unsafe { SetSecurityDescriptorDacl(sd_ptr, true, Some(acl as *const ACL), false) }
            .map_err(|e| PwError::Ipc(format!("SetSecurityDescriptorDacl: {}", e)))?;

        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: (&*sd as *const SECURITY_DESCRIPTOR) as *mut c_void,
            bInheritHandle: BOOL(0),
        };

        Ok(Self {
            sa,
            _sd: sd,
            _acl: acl_buf,
        })
    }

    fn as_ptr(&self) -> *mut c_void {
        &self.sa as *const SECURITY_ATTRIBUTES as *mut c_void
    }
}

/// 创建 一个 命名管道 服务器 (带 DACL).
///
/// `first` 为 true 时 使用 `first_pipe_instance` 做实例唯一性检测.
pub fn create_server(addr: &str, first: bool, acl: PipeAcl) -> Result<NamedPipeServer, PwError> {
    let sa = SecurityAttributes::new(acl)?;
    let mut opts = ServerOptions::new();
    opts.first_pipe_instance(first);

    // 调用 tokio 的 raw 接口 传入 SECURITY_ATTRIBUTES; `sa` 在整个调用期间保持存活.
    unsafe { opts.create_with_security_attributes_raw(addr, sa.as_ptr()) }
        .map_err(|e| PwError::Ipc(format!("create pipe `{}`: {}", addr, e)))
}

/// 检查 命名管道 是否存在 (不建立连接).
///
/// 使用 `WaitNamedPipeW` 超时 0: 仅当 返回 `ERROR_FILE_NOT_FOUND` 时 认为不存在;
/// 其它错误 (例如 权限不足 / 管道暂时忙) 表示 管道存在.
pub fn pipe_exists(pipe_name: &str) -> bool {
    let wide: Vec<u16> = OsStr::new(pipe_name)
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let ok = unsafe { WaitNamedPipeW(PCWSTR(wide.as_ptr()), 0) };
    if ok != BOOL(0) {
        return true;
    }
    io::Error::last_os_error().raw_os_error() != Some(ERROR_FILE_NOT_FOUND.0 as i32)
}

/// 创建 一个 已知 SID (返回 SID 字节).
fn well_known_sid(kind: WELL_KNOWN_SID_TYPE) -> Result<Vec<u8>, PwError> {
    let mut len = 0u32;
    // 首次调用 用于查询所需缓冲区大小, 按约定 必然返回 Err (ERROR_INSUFFICIENT_BUFFER),
    // 属 预期失败, 忽略.
    let _ = unsafe { CreateWellKnownSid(kind, None, None, &mut len) };

    let mut buf = vec![0u8; len as usize];
    unsafe {
        CreateWellKnownSid(
            kind,
            None,
            Some(PSID(buf.as_mut_ptr() as *mut c_void)),
            &mut len,
        )
    }
    .map_err(|e| PwError::Ipc(format!("CreateWellKnownSid: {}", e)))?;
    Ok(buf)
}

/// 读取 当前进程 令牌中的 用户 SID (返回 SID 字节).
fn current_user_sid() -> Result<Vec<u8>, PwError> {
    let process = unsafe { GetCurrentProcess() };
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
        .map_err(|e| PwError::Ipc(format!("OpenProcessToken: {}", e)))?;

    let mut needed = 0u32;
    // 首次调用 用于查询所需缓冲区大小, 按约定 必然返回 Err (ERROR_INSUFFICIENT_BUFFER),
    // 属 预期失败, 忽略.
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };

    let mut buf = vec![0u8; needed as usize];
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut c_void),
            needed,
            &mut needed,
        )
    }
    .map_err(|e| PwError::Ipc(format!("GetTokenInformation: {}", e)))?;

    // 关闭令牌句柄 失败 属 罕见异常, 且 无 后续处理 需要, 忽略.
    let _ = unsafe { CloseHandle(token) };

    let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
    let sid_len = unsafe { GetLengthSid(sid) } as usize;
    Ok(unsafe { slice::from_raw_parts(sid.0 as *const u8, sid_len) }.to_vec())
}
