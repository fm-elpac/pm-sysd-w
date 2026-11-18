//! Windows 平台 (pa) 实现.
//!
//! 主要支持平台.
//!
//! IPC 使用 命名管道 (Named pipe): 只允许本机程序连接 (默认 `reject_remote_clients`),
//! 禁止网络连接. 使用 `first_pipe_instance` 进行实例唯一性检测.
//!
//! 本模块只包含平台相关代码, 不做抽象/封装.

use std::{env, process, str, time::Duration};

use encoding_rs::{Decoder, DecoderResult};
use tokio::{
    net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer},
    process::Command,
    time::{Instant, sleep},
};
use windows::Win32::{
    Foundation::{ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND},
    System::Threading::CREATE_NO_WINDOW,
};

use crate::pa::{IpcAcl, SpawnOptions};

pub mod acl;
pub mod acp;
pub mod console;
pub mod service;

mod error;

#[cfg(test)]
mod test;

pub use error::PwError;

use acl::{PipeAcl, create_server, pipe_exists};
use acp::get_acp;

/// 客户端连接流
pub type ClientStream = NamedPipeClient;

/// 服务端连接流
pub type ServerStream = NamedPipeServer;

/// UTF-8 解码状态
enum Utf8Status {
    /// 整个缓冲区是合法且完整的 UTF-8
    Complete,
    /// 前 `usize` 字节是合法 UTF-8, 剩余字节是不完整字符的尾部
    Partial(usize),
    /// 整个缓冲区是不完整字符的前缀 (无完整字符)
    Incomplete,
    /// 缓冲区包含非法 UTF-8
    Invalid,
}

/// 解码器模式
enum Mode {
    /// 按 UTF-8 解码
    Utf8 {
        /// 待解码字节 (可能为空)
        pending: Vec<u8>,
    },
    /// 按系统 ACP (代码页) 解码
    Acp {
        decoder: Decoder,
        /// encoding_rs 解码器未消费的字节
        pending: Vec<u8>,
    },
}

/// 流式文本解码器.
///
/// 连续输入多个字节缓冲区, 解码结果按字符边界返回; 未构成完整字符的字节
/// 保留在解码器内部, 下次输入继续解码.
///
/// 首次按 UTF-8 解码; 遇到非法 UTF-8 时切换到 ACP 模式, 后续全部按 ACP
/// 解码. ACP 解码失败时退出 ACP 模式并返回错误.
pub struct TextDecoder {
    mode: Mode,
}

impl TextDecoder {
    /// 创建新的解码器 (UTF-8 模式)
    pub fn new() -> Self {
        Self {
            mode: Mode::Utf8 {
                pending: Vec::new(),
            },
        }
    }

    /// 输入一个字节缓冲区, 返回已解码的字符串
    pub fn decode(&mut self, bytes: &[u8]) -> Result<String, PwError> {
        if bytes.is_empty() {
            return Ok(String::new());
        }

        let mode = std::mem::replace(
            &mut self.mode,
            Mode::Utf8 {
                pending: Vec::new(),
            },
        );
        match mode {
            Mode::Utf8 { mut pending } => {
                pending.extend_from_slice(bytes);
                match utf8_status(&pending) {
                    Utf8Status::Complete => {
                        let s = str::from_utf8(&pending).expect("合法 UTF-8").to_string();
                        pending.clear();
                        self.mode = Mode::Utf8 { pending };
                        Ok(s)
                    }
                    Utf8Status::Partial(valid) => {
                        let s = str::from_utf8(&pending[..valid])
                            .expect("合法 UTF-8 前缀")
                            .to_string();
                        pending.drain(..valid);
                        self.mode = Mode::Utf8 { pending };
                        Ok(s)
                    }
                    Utf8Status::Incomplete => {
                        self.mode = Mode::Utf8 { pending };
                        Ok(String::new())
                    }
                    Utf8Status::Invalid => {
                        // UTF-8 无法解码: 整体切换到 ACP 模式, 并用 ACP 重新
                        // 解码当前待解码字节.
                        let mut decoder = acp_decoder()?;
                        let r = acp_decode(&mut decoder, &mut pending, false);
                        if r.is_ok() {
                            self.mode = Mode::Acp { decoder, pending };
                        } else {
                            // ACP 解码失败: 退出 ACP 模式, 丢弃所有数据.
                            self.mode = Mode::Utf8 {
                                pending: Vec::new(),
                            };
                        }
                        r
                    }
                }
            }
            Mode::Acp {
                mut decoder,
                mut pending,
            } => {
                pending.extend_from_slice(bytes);
                let r = acp_decode(&mut decoder, &mut pending, false);
                if r.is_ok() {
                    self.mode = Mode::Acp { decoder, pending };
                } else {
                    // ACP 解码失败: 退出 ACP 模式, 丢弃所有数据.
                    self.mode = Mode::Utf8 {
                        pending: Vec::new(),
                    };
                }
                r
            }
        }
    }

    /// 结束输入: 解码内部剩余字节.
    ///
    /// 解码完成后重置为 UTF-8 模式.
    pub fn finish(&mut self) -> Result<String, PwError> {
        let mode = std::mem::replace(
            &mut self.mode,
            Mode::Utf8 {
                pending: Vec::new(),
            },
        );
        match mode {
            Mode::Utf8 { pending } => {
                if pending.is_empty() {
                    return Ok(String::new());
                }
                match str::from_utf8(&pending) {
                    Ok(s) => Ok(s.to_string()),
                    Err(_) => {
                        // UTF-8 无法解码: 与 decode 一致, 切换到 ACP 模式并用
                        // ACP 解码剩余字节.
                        let mut decoder = acp_decoder()?;
                        let mut pending = pending;
                        acp_decode(&mut decoder, &mut pending, true)
                    }
                }
            }
            Mode::Acp {
                mut decoder,
                mut pending,
            } => acp_decode(&mut decoder, &mut pending, true),
        }
    }
}

/// 判断字节缓冲区的 UTF-8 解码状态
fn utf8_status(data: &[u8]) -> Utf8Status {
    match str::from_utf8(data) {
        Ok(_) => Utf8Status::Complete,
        Err(e) => {
            if e.error_len().is_some() {
                Utf8Status::Invalid
            } else if e.valid_up_to() > 0 {
                Utf8Status::Partial(e.valid_up_to())
            } else {
                Utf8Status::Incomplete
            }
        }
    }
}

/// 创建 ACP 解码器 (调用 `GetACP` 获取系统代码页)
fn acp_decoder() -> Result<Decoder, PwError> {
    let cp = get_acp();
    let enc = codepage::to_encoding_no_replacement(cp)
        .ok_or_else(|| PwError::Msg(format!("acp: unsupported code page {}", cp)))?;
    Ok(enc.new_decoder_without_bom_handling())
}

/// 使用 encoding_rs 解码器解码 `pending`, 返回已解码字符串.
///
/// `last` 为 true 时表示输入结束. 未消费字节保存在 `pending`.
fn acp_decode(decoder: &mut Decoder, pending: &mut Vec<u8>, last: bool) -> Result<String, PwError> {
    let mut out = String::new();
    loop {
        let data = std::mem::take(pending);
        let cap = decoder
            .max_utf8_buffer_length_without_replacement(data.len())
            .unwrap_or_else(|| data.len().saturating_mul(4).saturating_add(4));
        out.reserve(cap);

        let (result, read) = decoder.decode_to_string_without_replacement(&data, &mut out, last);
        match result {
            DecoderResult::InputEmpty => {
                if read < data.len() {
                    *pending = data[read..].to_vec();
                }
                return Ok(out);
            }
            DecoderResult::Malformed(_, _) => {
                // 失败前丢弃所有数据, 避免内部缓冲区无限膨胀.
                pending.clear();
                return Err(PwError::Msg("acp decode failed".to_string()));
            }
            DecoderResult::OutputFull => {
                *pending = data[read..].to_vec();
                continue;
            }
        }
    }
}

/// 命名管道监听器.
///
/// 使用 tokio 文档推荐的模式: 始终保有一个未连接的 server 实例, 避免客户端
/// 连接时出现 `NotFound`.
pub struct Listener {
    /// 完整管道地址
    addr: String,
    /// 管道 DACL 权限级别
    acl: PipeAcl,
    /// 当前等待连接的 server 实例
    pending: Option<NamedPipeServer>,
}

impl Listener {
    /// 创建命名管道监听器.
    ///
    /// 第一个实例使用 `first_pipe_instance(true)` 检测是否已有同名实例.
    pub fn bind(name: &str, acl: PipeAcl) -> Result<Self, PwError> {
        let addr = pipe_addr(name);
        let pending = create_server(&addr, true, acl)?;
        Ok(Self {
            addr,
            acl,
            pending: Some(pending),
        })
    }

    /// 接受下一个连接
    pub async fn accept(&mut self) -> Result<ServerStream, PwError> {
        let server = match self.pending.take() {
            Some(s) => s,
            None => {
                return Err(PwError::Ipc("pipe listener exhausted".to_string()));
            }
        };

        // 先创建替代 server (非 first), 保证连接窗口不中断
        match create_server(&self.addr, false, self.acl) {
            Ok(next) => self.pending = Some(next),
            Err(e) => {
                return Err(e);
            }
        }

        server
            .connect()
            .await
            .map_err(|e| PwError::Ipc(format!("pipe connect: {}", e)))?;
        Ok(server)
    }
}

/// 连接命名管道.
///
/// 快速失败: 总耗时最多 200ms. 服务器可能在替换 pipe 实例, 仅在
/// ERROR_PIPE_BUSY 时短暂重试; 不存在 / 权限不足直接返回.
pub async fn connect(name: &str) -> Result<ClientStream, PwError> {
    let pipe_name = pipe_addr(name);
    let deadline = Instant::now() + Duration::from_millis(200);
    loop {
        match ClientOptions::new().open(&pipe_name) {
            Ok(c) => return Ok(c),
            Err(e) => {
                let code = e.raw_os_error();
                if code == Some(ERROR_FILE_NOT_FOUND.0 as i32) {
                    return Err(PwError::NotFound(format!(
                        "connect pipe `{}`: not found",
                        pipe_name
                    )));
                }
                if code == Some(ERROR_ACCESS_DENIED.0 as i32) {
                    return Err(PwError::Permission(format!(
                        "connect pipe `{}`: access denied",
                        pipe_name
                    )));
                }
                if Instant::now() >= deadline {
                    return Err(PwError::Ipc(format!(
                        "connect pipe `{}`: os error {:?}",
                        pipe_name, code
                    )));
                }
            }
        }
        sleep(Duration::from_millis(20)).await;
    }
}

/// 检查命名管道是否存在
pub fn exists(name: &str) -> bool {
    pipe_exists(&pipe_addr(name))
}

/// 平台特定的进程创建标志
pub fn spawn_flags(cmd: &mut Command, opts: SpawnOptions) {
    if opts.create_no_window {
        cmd.creation_flags(CREATE_NO_WINDOW.0);
    }
}

/// 重启自身 (soft-reboot): 启动新进程后当前进程自行退出
pub fn soft_reboot(args: &[String]) -> Result<(), PwError> {
    let exe = env::current_exe().map_err(|e| PwError::Msg(format!("current_exe: {}", e)))?;
    match process::Command::new(&exe).args(args).spawn() {
        Ok(_) => Ok(()),
        Err(e) => Err(PwError::Msg(format!("soft-reboot spawn: {}", e))),
    }
}

/// `IpcAcl` -> `PipeAcl`
impl From<IpcAcl> for PipeAcl {
    fn from(acl: IpcAcl) -> Self {
        match acl {
            IpcAcl::System => PipeAcl::System,
            IpcAcl::SystemReadOnly => PipeAcl::SystemReadOnly,
            IpcAcl::CurrentUser => PipeAcl::CurrentUser,
        }
    }
}

/// 命名管道完整地址 (`\\.\pipe\NAME`)
fn pipe_addr(name: &str) -> String {
    format!(r"\\.\pipe\{}", name)
}
