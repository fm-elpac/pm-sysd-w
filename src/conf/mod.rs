//! 配置文件解析/加载.
//!
//! pm-sysd-w 使用 pm-tc 的集中式 toml 配置, 把多个 unit 写在一个文件中.
//! 本模块调用 pm-tc 的 `task::plan("sysd")`, 从 `PlanDebug` 中取出所有 unit 配置.
//!
//! 配置格式参考 `doc/user/config.md`.

pub mod limit;

mod def;
mod error;
mod load;

pub use def::{Conf, Install, RestartPolicy, Service, ServiceType, Unit, UnitType, Win};
pub use error::ConfError;
pub use load::{MULTI_LINE_PREFIXES, SUPPORTED_KEYS, load_conf};
