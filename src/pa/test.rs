//! pa 模块测试 (顶层通用项).

use std::path::{Path, PathBuf};

use super::{TextDecoder, resolve_path};

#[test]
fn resolve_path_relative() {
    let base = Path::new("/a/b");
    assert_eq!(resolve_path(base, "c.db"), PathBuf::from("/a/b/c.db"));
    assert_eq!(resolve_path(base, "/c.db"), PathBuf::from("/c.db"));
}

#[test]
fn text_decoder_utf8() {
    let mut d = TextDecoder::new();
    assert_eq!(d.decode("你好".as_bytes()).unwrap(), "你好");
    assert_eq!(d.decode(b"abc").unwrap(), "abc");
    assert_eq!(d.finish().unwrap(), "");
}

#[test]
fn text_decoder_stream_split_char() {
    let mut d = TextDecoder::new();
    let b = "你".as_bytes();
    assert_eq!(b.len(), 3);

    // 前 2 字节不构成完整字符, 应留到下次.
    assert_eq!(d.decode(&b[..2]).unwrap(), "");
    assert_eq!(d.decode(&b[2..]).unwrap(), "你");
    assert_eq!(d.finish().unwrap(), "");
}

#[cfg(target_os = "linux")]
#[test]
fn text_decoder_utf8_invalid() {
    let mut d = TextDecoder::new();
    // Linux 固定按 UTF-8, 非法字节应报错.
    assert!(d.decode(b"\xff").is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn text_decoder_utf8_incomplete_finish() {
    let mut d = TextDecoder::new();
    let b = "你".as_bytes();
    assert_eq!(d.decode(&b[..2]).unwrap(), "");
    assert!(d.finish().is_err());
}
