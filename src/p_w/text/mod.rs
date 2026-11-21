//! p_w::text: 字符串流式解码 (UTF-8 / 系统 ACP).

use std::str;

use encoding_rs::{Decoder, DecoderResult};

use super::PwError;
use super::acp::get_acp;

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

#[cfg(test)]
mod test;
