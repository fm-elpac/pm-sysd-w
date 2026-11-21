//! p_w::text 测试.

use super::super::acp::get_acp;
use super::TextDecoder;

#[test]
fn acp_fallback_decodes_current_acp() {
    let cp = get_acp();
    let enc = codepage::to_encoding_no_replacement(cp).expect("当前 ACP 应有 encoding_rs 映射");

    // 后面追加 ASCII 字节, 使前面的非 UTF-8 ACP 字节序列成为非法 UTF-8,
    // 从而触发 UTF-8 -> ACP 切换.
    let (bytes, _, _) = enc.encode("éa");
    let mut d = TextDecoder::new();
    assert_eq!(d.decode(&bytes).unwrap(), "éa");
}

#[test]
fn acp_fallback_finish_decodes_pending() {
    let cp = get_acp();
    let enc = codepage::to_encoding_no_replacement(cp).expect("当前 ACP 应有 encoding_rs 映射");

    // 单个 `é` 在常见非 UTF-8 ACP 中可能只是 UTF-8 的不完整前缀,
    // 因此 decode 返回空字符串, finish 应切换 ACP 并解码剩余字节.
    let (bytes, _, _) = enc.encode("é");
    let mut d = TextDecoder::new();
    assert_eq!(d.decode(&bytes).unwrap(), "");
    assert_eq!(d.finish().unwrap(), "é");
}
