//! Windows 命名管道 ACL 辅助 (安全描述符).
//!
//! 用于给 pm-sysd-init 创建的命名管道设置 DACL, 防止越权访问:
//!
//! - [`PipeAcl::System`]: 仅管理员 / SYSTEM.
//! - [`PipeAcl::SystemReadOnly`]: 本机普通用户 可连接 (读+写), 只读限制由 init
//!   在命令层执行.
//! - [`PipeAcl::CurrentUser`]: 仅当前用户.
//!
//! Windows 安全描述符 / ACL API 均为 `unsafe fn`,
//! 本模块是 p_w 中允许 `unsafe` 的模块之一.
//! 把 unsafe 调用集中在此处, 对外提供安全接口.
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

/// 安全描述符修订号 (固定)
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

/// 命名管道权限级别
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeAcl {
    /// 系统级主管道: 仅管理员 / SYSTEM
    System,
    /// 系统级只读管道: 本机普通用户可连接 (读+写), 只读限制由 init 命令层执行
    SystemReadOnly,
    /// 用户级 / 自定义实例: 仅当前用户
    CurrentUser,
}

/// 保持安全描述符 / ACL 缓冲区存活, 并提供传递给 `CreateNamedPipeW` 的指针
struct SecurityAttributes {
    sa: SECURITY_ATTRIBUTES,
    _sd: Box<SECURITY_DESCRIPTOR>,
    _acl: AlignedBuffer,
}

/// 按 `usize` 对齐的字节缓冲区, 供 Win32 API 写入结构体
struct AlignedBuffer {
    data: Vec<usize>,
    len: usize,
}

impl AlignedBuffer {
    fn new_zeroed(len: usize) -> Self {
        Self {
            data: vec![0usize; len.div_ceil(size_of::<usize>())],
            len,
        }
    }

    fn from_slice(bytes: &[u8]) -> Self {
        let mut buf = Self::new_zeroed(bytes.len());
        unsafe { buf.as_mut_ptr().copy_from(bytes.as_ptr(), bytes.len()) };
        buf
    }

    fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr() as *const u8
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.data.as_mut_ptr() as *mut u8
    }

    fn len(&self) -> usize {
        self.len
    }
}

/// 自动关闭 Win32 句柄
struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        // `Drop` 无法返回错误, 关闭句柄失败只能忽略
        let _ = unsafe { CloseHandle(self.0) };
    }
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
                // 允许普通用户读+写以完成请求/响应通信. 只读由 init 命令层限制.
                vec![FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0],
            ),
            PipeAcl::CurrentUser => (vec![current_user_sid()?], vec![FILE_ALL_ACCESS.0]),
        };

        // ACL 缓冲区 (足够容纳少量 ACE)
        let mut acl_buf = AlignedBuffer::new_zeroed(1024);
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

/// 创建一个命名管道服务器 (带 DACL).
///
/// `first` 为 true 时使用 `first_pipe_instance` 做实例唯一性检测.
pub fn create_server(addr: &str, first: bool, acl: PipeAcl) -> Result<NamedPipeServer, PwError> {
    let sa = SecurityAttributes::new(acl)?;
    let mut opts = ServerOptions::new();
    opts.first_pipe_instance(first);

    // 调用 tokio 的 raw 接口传入 SECURITY_ATTRIBUTES; `sa` 在整个调用期间保持存活.
    unsafe { opts.create_with_security_attributes_raw(addr, sa.as_ptr()) }
        .map_err(|e| PwError::Ipc(format!("create pipe `{}`: {}", addr, e)))
}

/// 检查命名管道是否存在 (不建立连接).
///
/// 使用 `WaitNamedPipeW` 超时 0: 仅当返回 `ERROR_FILE_NOT_FOUND` 时认为不存在;
/// 其它错误 (例如权限不足 / 管道暂时忙) 表示管道存在.
///
/// 已知限制 (低严重度): 空名称 / `\\.\pipe\` 等非法地址可能返回
/// `ERROR_BAD_PATHNAME` / `ERROR_INVALID_PARAMETER`, 因而被误判为存在.
/// 本程序只把此函数作为连接前的快速失败检查, 正常实例名由 `Instance::name`
/// 生成并带固定前缀; 即使异常自定义实例名触发误判, 后续 `connect` 仍会失败
/// 并返回错误, 因此在本程序的使用场景下不会导致问题.
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

/// 创建一个已知 SID (返回对齐的 SID 字节)
fn well_known_sid(kind: WELL_KNOWN_SID_TYPE) -> Result<AlignedBuffer, PwError> {
    let mut len = 0u32;
    // 首次调用用于查询所需缓冲区大小, 按约定必然返回 Err (ERROR_INSUFFICIENT_BUFFER),
    // 属预期失败, 忽略.
    let _ = unsafe { CreateWellKnownSid(kind, None, None, &mut len) };

    let mut buf = AlignedBuffer::new_zeroed(len as usize);
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

/// 读取当前进程令牌中的用户 SID (返回对齐的 SID 字节)
fn current_user_sid() -> Result<AlignedBuffer, PwError> {
    let process = unsafe { GetCurrentProcess() };
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
        .map_err(|e| PwError::Ipc(format!("OpenProcessToken: {}", e)))?;
    let token = HandleGuard(token);

    let mut needed = 0u32;
    // 首次调用用于查询所需缓冲区大小, 按约定必然返回 Err (ERROR_INSUFFICIENT_BUFFER),
    // 属预期失败, 忽略.
    let _ = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut needed) };

    let mut buf = AlignedBuffer::new_zeroed(needed as usize);
    unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut c_void),
            needed,
            &mut needed,
        )
    }
    .map_err(|e| PwError::Ipc(format!("GetTokenInformation: {}", e)))?;

    let sid = unsafe { (*(buf.as_ptr() as *const TOKEN_USER)).User.Sid };
    let sid_len = unsafe { GetLengthSid(sid) } as usize;
    Ok(AlignedBuffer::from_slice(unsafe {
        slice::from_raw_parts(sid.0 as *const u8, sid_len)
    }))
}
