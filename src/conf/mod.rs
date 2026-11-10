//! 配置文件 解析/加载.
//!
//! pm-sysd-w 使用 pm-tc 的集中式 toml 配置, 把多个 unit 写在一个文件中. 本模块
//! 调用 pm-tc 的 `task::plan("sysd")`, 从 `PlanDebug` 中取出所有 unit 配置.
//!
//! 配置格式参考 `doc/user/config.md`.

mod error;
#[cfg(test)]
mod test;

pub use error::ConfError;

use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use pm_tc::{
    conf::{ReadConfFile, SrcInfo, load},
    task::{self, CheckEnv},
};

/// service 启动类型.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceType {
    /// 默认: 启动即视为 active.
    Simple,
    /// 进程退出 (成功) 视为启动完成, 不持续运行.
    Oneshot,
}

impl ServiceType {
    /// 配置关键字 (用于 显示/序列化).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Oneshot => "oneshot",
        }
    }
}

/// 重启策略.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    /// 不自动重启 (默认).
    No,
    /// 退出后总是重启.
    Always,
    /// 仅异常退出时重启.
    OnFailure,
}

impl RestartPolicy {
    /// 配置关键字 (用于 显示/序列化).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Always => "always",
            Self::OnFailure => "on-failure",
        }
    }
}

/// `[Service]` 节 配置.
#[derive(Debug, Clone)]
pub struct Service {
    /// 启动类型
    pub service_type: ServiceType,
    /// 启动命令
    pub exec_start: String,
    /// 停止命令
    pub exec_stop: Option<String>,
    /// reload 命令
    pub exec_reload: Option<String>,
    /// 重启策略
    pub restart: RestartPolicy,
    /// 重启间隔 (秒)
    pub restart_sec: u32,
    /// 启动超时 (秒)
    pub timeout_start_sec: u32,
    /// 停止超时 (秒)
    pub timeout_stop_sec: u32,
    /// 工作目录
    pub working_directory: Option<String>,
    /// 环境变量 (`KEY=VALUE` 列表)
    pub environment: Vec<(String, String)>,
    /// 进程退出后是否仍视为 active
    pub remain_after_exit: bool,
}

/// `[Install]` 节 配置.
#[derive(Debug, Clone)]
pub struct Install {
    /// 所属 target (显式设置后该 unit 才可 enable)
    pub wanted_by: Option<String>,
}

/// `[Win]` 节 配置 (pm-sysd-w 专属扩展).
#[derive(Debug, Clone, Copy)]
pub struct Win {
    /// 启动进程时 设置 CREATE_NO_WINDOW 标志 (console 程序不显示 黑窗口).
    ///
    /// 仅 Windows 平台有效, Linux 上 忽略.
    pub create_no_window: bool,
}

/// 一个 unit (service) 的配置 (模仿 systemd unit 文件 的 节).
#[derive(Debug, Clone)]
pub struct Unit {
    /// unit 名称 (例如 `test-run.service`)
    pub name: String,
    /// 描述
    pub description: String,
    /// `[Service]` 节
    pub service: Service,
    /// `[Install]` 节
    pub install: Install,
    /// `[Win]` 节 (pm-sysd-w 专属扩展)
    pub win: Win,
}

/// 加载配置文件 得到的结果.
#[derive(Debug, Clone)]
pub struct Conf {
    /// 所有 unit (service) 配置
    pub units: Vec<Unit>,
    /// sqlite 数据库路径 (绝对路径)
    pub db_path: String,
    /// 主配置文件所在目录 (绝对路径)
    pub conf_dir: String,
}

/// 读取配置文件 的 默认实现.
struct FsReader;

impl ReadConfFile for FsReader {
    fn read(&mut self, path: &str) -> Result<String, Box<dyn Error>> {
        Ok(fs::read_to_string(path)?)
    }
}

/// 检查环境现状 的 默认实现.
struct FsChecker;

impl CheckEnv for FsChecker {
    fn file_exist(&mut self, path: &str) -> Result<bool, Box<dyn Error>> {
        Ok(Path::new(path).exists())
    }

    fn file_read(&mut self, path: &str) -> Result<String, Box<dyn Error>> {
        Ok(fs::read_to_string(path)?)
    }
}

/// 加载集中式配置文件 (路径 `conf_path`), 返回其中的所有 unit 与 数据库路径.
///
/// 内部调用 pm-tc 的 `task::plan("sysd")`.
pub fn load_conf(conf_path: &str) -> Result<Conf, ConfError> {
    let conf_dir = Path::new(conf_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // (环境) 变量: 内置变量 + 系统环境变量 (与 pm-tc CLI 一致).
    let mut env = task::env_builtin();
    env.add(task::env_sys());

    // 加载 配置文件.
    let mut src = SrcInfo::new();
    let mut rc = FsReader;
    let c = load(conf_path, &mut rc, &mut src).map_err(|e| ConfError::Pm(e.to_string()))?;

    // 计划执行 (拿到 所有 `[[t.sysd]]` 实例的 最终变量).
    let mut ce = FsChecker;
    let task = vec!["sysd".to_string()];
    let (_, pd) = task::plan(&task, &c, &env, &mut ce).map_err(|e| ConfError::Pm(e.to_string()))?;

    // 数据库 路径 (`[env] SYSD_DB`).
    let db_rel = pd
        .env0
        .get("SYSD_DB")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "pm_sysd_w_v1.db".to_string());
    let db_path = resolve_path(&conf_dir, &db_rel);

    // 解析 每个 unit.
    let mut units = Vec::new();
    for t in &pd.task {
        units.push(parse_unit(&t.env)?);
    }

    Ok(Conf {
        units,
        db_path,
        conf_dir: conf_dir.to_string_lossy().into_owned(),
    })
}

/// 从 pm-tc 任务 的 最终变量 解析出 一个 [`Unit`].
///
/// 变量名即 `env:XXX` 去掉前缀后的名称 (例如 `Unit`, `Service.ExecStart`).
fn parse_unit(env: &BTreeMap<String, String>) -> Result<Unit, ConfError> {
    let name = get_req(env, "Unit")?;
    if !name.ends_with(".service") {
        return Err(ConfError::BadUnit(format!(
            "unit name `{}` must end with `.service`",
            name
        )));
    }

    let description = get(env, "Unit.Description").unwrap_or("").to_string();

    let service_type = match get(env, "Service.Type").unwrap_or("simple") {
        "simple" => ServiceType::Simple,
        "oneshot" => ServiceType::Oneshot,
        other => {
            return Err(ConfError::BadUnit(format!(
                "unit `{}`: unknown Service.Type `{}`",
                name, other
            )));
        }
    };

    let exec_start = get_req(env, "Service.ExecStart")?;

    let restart = match get(env, "Service.Restart").unwrap_or("no") {
        "no" => RestartPolicy::No,
        "always" => RestartPolicy::Always,
        "on-failure" => RestartPolicy::OnFailure,
        other => {
            return Err(ConfError::BadUnit(format!(
                "unit `{}`: unknown Service.Restart `{}`",
                name, other
            )));
        }
    };

    let win = Win {
        create_no_window: get_bool(env, "Win.CreateNoWindow", false)?,
    };

    Ok(Unit {
        name,
        description,
        service: Service {
            service_type,
            exec_start,
            exec_stop: get_nonempty(env, "Service.ExecStop"),
            exec_reload: get_nonempty(env, "Service.ExecReload"),
            restart,
            restart_sec: get_u32(env, "Service.RestartSec", "1")?,
            timeout_start_sec: get_u32(env, "Service.TimeoutStartSec", "90")?,
            timeout_stop_sec: get_u32(env, "Service.TimeoutStopSec", "90")?,
            working_directory: get_nonempty(env, "Service.WorkingDirectory"),
            environment: parse_environment(get(env, "Service.Environment"))?,
            remain_after_exit: get_bool(env, "Service.RemainAfterExit", false)?,
        },
        install: Install {
            wanted_by: get_nonempty(env, "Install.WantedBy"),
        },
        win,
    })
}

/// 读取 变量值.
fn get<'a>(env: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    env.get(key).map(|s| s.as_str())
}

/// 读取 必需的 变量值 (非空).
fn get_req(env: &BTreeMap<String, String>, key: &str) -> Result<String, ConfError> {
    match get_nonempty(env, key) {
        Some(v) => Ok(v),
        None => Err(ConfError::BadUnit(format!("missing `{}`", key))),
    }
}

/// 读取 非空 变量值.
fn get_nonempty(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    get(env, key)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// 读取 非负整数 变量值 (带 默认值).
fn get_u32(env: &BTreeMap<String, String>, key: &str, default: &str) -> Result<u32, ConfError> {
    let s = get(env, key).unwrap_or(default);
    s.parse::<u32>().map_err(|_| {
        ConfError::BadUnit(format!("`{}` must be a non-negative integer: `{}`", key, s))
    })
}

/// 读取 布尔 变量值 (带 默认值).
///
/// 支持 systemd 布尔语法: `1` / `yes` / `true` / `on` 为 true, `0` / `no` /
/// `false` / `off` 为 false.
fn get_bool(env: &BTreeMap<String, String>, key: &str, default: bool) -> Result<bool, ConfError> {
    let s = match get(env, key) {
        Some(s) => s,
        None => return Ok(default),
    };
    match s {
        "yes" | "true" | "1" | "on" => Ok(true),
        "no" | "false" | "0" | "off" => Ok(false),
        _ => Err(ConfError::BadUnit(format!(
            "`{}` must be a boolean: `{}`",
            key, s
        ))),
    }
}

/// 解析 `KEY=VALUE` 环境变量 (单个字符串).
fn parse_environment(value: Option<&str>) -> Result<Vec<(String, String)>, ConfError> {
    let Some(v) = value else {
        return Ok(Vec::new());
    };
    if v.is_empty() {
        return Ok(Vec::new());
    }
    match v.split_once('=') {
        Some((k, val)) if !k.is_empty() => Ok(vec![(k.to_string(), val.to_string())]),
        _ => Err(ConfError::BadUnit(format!(
            "environment must be `KEY=VALUE`: `{}`",
            v
        ))),
    }
}

/// 把 相对路径 解析为 相对 `base` 的 绝对路径.
fn resolve_path(base: &Path, p: &str) -> String {
    let path = Path::new(p);
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    full.to_string_lossy().into_owned()
}
