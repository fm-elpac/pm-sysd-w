//! 命令行程序 (CLI) 模块.
//!
//! 仅启用 `cli` feature 时编译.
//!
//! 包含 3 个二进制程序 的 CLI 实现:
//!
//! - [`init`]: pm-sysd-init 程序.
//! - [`ctl`]: pm-sysd-ctl 程序.
//! - [`log`]: pm-sysd-log 程序.
//!
//! 3 个程序 共用的 实例解析 / 连接 / 握手 等逻辑也放在本模块.

use crate::{
    jsonl::{Hello, check_version, decode_hello, encode_hello},
    pa::{Instance, SysdClient, ipc_exists},
};

#[cfg(feature = "cli_init")]
pub mod init;

#[cfg(feature = "cli_ctl")]
pub mod ctl;

#[cfg(feature = "cli_log")]
pub mod log;

/// 设置 实例 (禁止 重复指定).
///
/// 成功返回 `Ok(())`; 已指定实例时 返回 错误消息.
pub fn set_instance(slot: &mut Option<Instance>, value: Instance) -> Result<(), String> {
    if slot.is_some() {
        return Err("--system / --user / --machine=NAME: only one may be specified".into());
    }
    *slot = Some(value);
    Ok(())
}

/// unit 名称 未写后缀时 自动补 `.service`.
///
/// 名称 已含 `.` 时 视为完整名称, 原样返回 (与 systemd 的规则一致).
pub fn complete_unit(name: &str) -> String {
    if name.contains('.') {
        name.to_string()
    } else {
        format!("{}.service", name)
    }
}

/// 解析 实际连接的实例.
///
/// - 显式指定: 直接使用.
/// - 未指定: 优先 `--system`; 若 `--system` 不存在, 回退 `--user`.
pub fn resolve_instance(explicit: Option<Instance>) -> Result<Instance, String> {
    resolve_instance_with(ipc_exists, explicit)
}

/// 测试用: 使用 自定义 `exists` 谓词 解析实例.
pub fn resolve_instance_with(
    exists: impl Fn(&str) -> bool,
    explicit: Option<Instance>,
) -> Result<Instance, String> {
    match explicit {
        Some(i) => Ok(i),
        None => {
            let system = Instance::System;
            let user = Instance::User;
            if exists(&system.transport()) {
                Ok(system)
            } else if exists(&user.transport()) {
                Ok(user)
            } else {
                Err("no pm-sysd-init instance found (tried --system, then --user)".into())
            }
        }
    }
}

/// 连接 指定实例 并完成 版本握手.
///
/// 内部先检查 IPC 端点是否存在, 再进行连接 (快速失败); 返回 可用于收发 jsonl
/// 消息 的 [`SysdClient`].
pub async fn connect(instance: &Instance) -> Result<SysdClient, String> {
    let mut client = SysdClient::connect(instance)
        .await
        .map_err(|e| e.to_string())?;

    let line = encode_hello(&Hello::new()).map_err(|e| e.to_string())?;
    client.send(line).await.map_err(|e| e.to_string())?;

    let line = client.recv().await.map_err(|e| e.to_string())?;
    let hello = decode_hello(&line).map_err(|e| e.to_string())?;
    check_version(&hello).map_err(|e| e.to_string())?;

    Ok(client)
}
