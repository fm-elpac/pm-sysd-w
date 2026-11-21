//! pm-sysd-init --reload 子进程入口.

use std::env;

use crate::conf::{limit::SysdLimit, load_conf};

#[cfg(test)]
mod test;

/// pm-sysd-init --reload 命令行入口.
///
/// 从环境变量 `PM_SYSD_CONF` 读取主配置文件路径, 调用 pm-tc 加载配置,
/// 把结果 JSON 写入 stdout 后退出.
pub fn run_reload(limit: &SysdLimit) -> Result<(), String> {
    let conf_path = env::var("PM_SYSD_CONF")
        .map_err(|_| "PM_SYSD_CONF environment variable is not set".to_string())?;
    let json = run_reload_conf(&conf_path, limit)?;
    println!("{}", json);
    Ok(())
}

/// 执行 --reload 模式核心: 只加载 toml 配置并序列化为 JSON, 不访问数据库.
///
/// 拆成独立函数便于测试 (直接传入路径, 不依赖环境变量).
fn run_reload_conf(conf_path: &str, limit: &SysdLimit) -> Result<String, String> {
    if conf_path.is_empty() {
        return Err("PM_SYSD_CONF is empty".to_string());
    }

    let conf = load_conf(conf_path, limit).map_err(|e| e.to_string())?;
    let json = serde_json::to_string(&conf).map_err(|e| e.to_string())?;
    if json.len() > limit.conf_total_byte {
        return Err(format!(
            "config JSON output exceeds {} bytes",
            limit.conf_total_byte
        ));
    }
    Ok(json)
}
