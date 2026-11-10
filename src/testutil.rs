//! 测试 公共工具 (临时目录).

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static N: AtomicU64 = AtomicU64::new(0);

fn uid() -> u64 {
    N.fetch_add(1, Ordering::Relaxed)
}

/// 系统临时目录 的 基准目录.
///
/// - Linux: `/tmp` (系统重启后 自动消失).
/// - Windows: `%TEMP%` (系统临时目录).
/// - 其它平台: `std::env::temp_dir()`.
fn temp_base() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/tmp")
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::env::temp_dir()
    }
}

/// 自动清理 的 临时目录 (RAII guard).
///
/// 位于 系统临时目录; `Drop` 时 递归删除 整个目录 (正常结束 与 panic 展开 都会触发),
/// 从而 避免 对系统留下 永久副作用.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// 在 系统临时目录 中 新建一个 唯一 子目录.
    pub fn new(prefix: &str) -> Self {
        let path = temp_base().join(format!(
            "pm_sysd_w_{}_{}_{}",
            prefix,
            std::process::id(),
            uid()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    /// 目录 路径.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
