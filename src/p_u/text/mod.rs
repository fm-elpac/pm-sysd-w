//! p_u::text: 字符串流式解码 (UTF-8).

use std::str;

use super::PuError;

/// 流式 UTF-8 解码器.
///
/// 连续输入多个字节缓冲区, 解码结果按字符边界返回; 未构成完整字符的字节
/// 保留在解码器内部, 下次输入继续解码.
pub struct TextDecoder {
    /// 待解码字节 (可能为空)
    pending: Vec<u8>,
}

impl TextDecoder {
    /// 创建新的 UTF-8 解码器
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// 输入一个字节缓冲区, 返回已解码的 UTF-8 字符串.
    ///
    /// 若缓冲区以不完整的多字节字符结尾, 只返回前面完整字符, 剩余字节留到
    /// 下次. 若缓冲区包含非法 UTF-8, 返回错误并丢弃所有数据.
    pub fn decode(&mut self, bytes: &[u8]) -> Result<String, PuError> {
        if bytes.is_empty() {
            return Ok(String::new());
        }

        self.pending.extend_from_slice(bytes);
        let data = std::mem::take(&mut self.pending);
        match str::from_utf8(&data) {
            Ok(s) => Ok(s.to_string()),
            Err(e) => {
                let valid = e.valid_up_to();
                if valid > 0 {
                    let s = str::from_utf8(&data[..valid])
                        .expect("valid_up_to 保证前缀是合法 UTF-8")
                        .to_string();
                    self.pending.extend_from_slice(&data[valid..]);
                    Ok(s)
                } else if e.error_len().is_some() {
                    // 解码失败: 返回错误前丢弃所有数据, 避免内部缓冲区膨胀
                    self.pending.clear();
                    Err(PuError::Msg(format!("invalid utf-8: {}", e)))
                } else {
                    // 整个缓冲区是不完整字符的前缀, 全部保留待后续输入
                    self.pending = data;
                    Ok(String::new())
                }
            }
        }
    }

    /// 结束输入: 解码内部剩余字节.
    ///
    /// 剩余字节为空时返回空字符串. 否则必须是完整合法 UTF-8, 失败返回错误.
    pub fn finish(&mut self) -> Result<String, PuError> {
        let data = std::mem::take(&mut self.pending);
        if data.is_empty() {
            return Ok(String::new());
        }
        match str::from_utf8(&data) {
            Ok(s) => Ok(s.to_string()),
            Err(e) => Err(PuError::Msg(format!("invalid utf-8 at end: {}", e))),
        }
    }
}
