//! 限速警告 (rate-limited warning).

use std::{
    collections::{HashMap, hash_map::Entry},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use log::warn;

/// 丢弃日志警告的最小间隔 (速率限制)
pub const WARN_RATE_LIMIT: Duration = Duration::from_secs(1);

/// 限速警告: 按 `key` 分别限速, 同一个 `key` 至少间隔 [`WARN_RATE_LIMIT`]
/// 才输出一次警告.
///
/// `key` 必须是 **固定** 的字符串常量 (编译期确定), 不能每次调用都不同
/// (例如不要包含时间戳 / 计数器 / 具体错误内容). 若 `key` 动态变化, 会不断
/// 产生新 key 导致内存膨胀; 自动清理只是兜底, 不能作为依赖.
pub fn warn_rate_limited(key: &'static str, msg: &str) {
    static STATE: OnceLock<Mutex<RateLimitState>> = OnceLock::new();
    let state = STATE.get_or_init(|| Mutex::new(RateLimitState::default()));
    let mut guard = state.lock().unwrap_or_else(|e| e.into_inner());
    guard.process(key, msg, Instant::now());
}

/// 限速警告状态 (不依赖全局, 便于测试)
#[derive(Debug, Default)]
pub(super) struct RateLimitState {
    /// 每个 key 最近一次实际输出警告的时间 (用于限速判定)
    pub(super) keys: HashMap<&'static str, Instant>,
    /// 最近一次执行清理的时间 (对清理动作本身限速)
    last_cleanup: Option<Instant>,
}

impl RateLimitState {
    /// 处理一次限速警告, 返回是否实际输出
    pub(super) fn process(&mut self, key: &'static str, msg: &str, now: Instant) -> bool {
        let allow = match self.keys.entry(key) {
            Entry::Vacant(e) => {
                // 首次见到该 key: 直接输出
                e.insert(now);
                true
            }
            Entry::Occupied(mut e) => {
                let last = *e.get();
                let allow = now.duration_since(last) >= WARN_RATE_LIMIT;
                if allow {
                    *e.get_mut() = now;
                }
                allow
            }
        };

        if allow {
            warn!("{}", msg);
        }

        self.cleanup(now);
        allow
    }

    /// 自动清理 (兜底): 对清理动作本身限速, 距上次清理超过 2 倍限速间隔才做
    /// 一次清理, 丢弃已过期 key
    fn cleanup(&mut self, now: Instant) {
        // 清理动作本身限速, 避免每次调用都遍历所有 key
        if let Some(last) = self.last_cleanup {
            if now.duration_since(last) <= WARN_RATE_LIMIT * 2 {
                return;
            }
        }
        // 丢弃过期 key: 超过 1 倍限速间隔未输出 (限速窗口已过, 可安全删除)
        self.keys
            .retain(|_, last| now.duration_since(*last) <= WARN_RATE_LIMIT);
        self.last_cleanup = Some(now);
    }
}
