//! systemd 风格的打印输出 / 字符串格式化.
//!
//! 本模块存放模仿 systemd 控制台输出的小工具.

/// 构造 systemd 风格的 "Reached target ..." 控制台消息.
///
/// systemd 的 console 成功行格式为 `[  OK  ] Reached target %s.`
/// (状态词 `"  OK  "` 前后各有两个空格, 消息末尾有句号).
pub fn reached_target(target: &str) -> String {
    format!("[  OK  ] Reached target {}.", target)
}
