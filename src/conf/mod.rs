//! 配置文件 解析/加载.
//!
//! pm-sysd-w 使用 pm-tc 的集中式 toml 配置, 把多个 unit 写在一个文件中. 本模块
//! 调用 pm-tc 的 `task::plan("sysd")`, 从 `PlanDebug` 中取出所有 unit 配置.
//!
//! 配置格式参考 `doc/user/config.md`.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use pm_tc::{
    conf::{HasP, ReadConfFile, SrcInfo, load},
    task::{CheckEnv, env_builtin, plan},
    te::{PmtcEnv, exec_eval_2},
};

mod error;
mod types;

#[cfg(test)]
mod test;

pub use error::ConfError;
pub use types::{Conf, Install, RestartPolicy, Service, ServiceType, Unit, UnitType, Win};

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

    // (环境) 变量: 内置变量
    let env = env_builtin();
    // pm-sysd-w 不使用 环境变量 (当前版本)
    //env.add(env_sys());

    // 加载 配置文件.
    let mut src = SrcInfo::new();
    let mut rc = FsReader;
    let c = load(conf_path, &mut rc, &mut src).map_err(|e| ConfError::Pm(e.to_string()))?;

    // 计划执行 (拿到 所有 `[[t.sysd]]` 实例的 最终变量).
    // 空配置 (没有 任何 `[[t.sysd]]`) 也允许加载: 此时 传 空 task 列表.
    let sysd_tasks = c.task.get("sysd").map(|v| v.as_slice()).unwrap_or(&[]);
    let task: Vec<String> = if sysd_tasks.is_empty() {
        Vec::new()
    } else {
        vec!["sysd".to_string()]
    };
    let mut ce = FsChecker;
    let (_, pd) = plan(&task, &c, &env, &mut ce).map_err(|e| ConfError::Pm(e.to_string()))?;

    // 数据库 路径 (`[env] SYSD_DB`).
    let db_rel = pd
        .env0
        .get("SYSD_DB")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "pm_sysd_w_v1.db".to_string());
    let db_path = resolve_path(&conf_dir, &db_rel);

    // 解析 每个 unit (命令字段 使用 pm-tc exec_eval 切分).
    let mut units = Vec::new();
    let mut warnings = Vec::new();
    for (t, ht) in pd.task.iter().zip(sysd_tasks.iter()) {
        units.push(parse_unit(
            &t.env,
            &ht.as_ref().env,
            c.eval_rec,
            &mut warnings,
        )?);
    }

    // 检测 重复 unit 名称 (名称 重复 视为 配置错误).
    let mut seen = HashSet::new();
    for u in &units {
        if !seen.insert(u.name.as_str()) {
            return Err(ConfError::BadUnit(format!("duplicate unit `{}`", u.name)));
        }
    }

    Ok(Conf {
        units,
        db_path,
        conf_dir: conf_dir.to_string_lossy().into_owned(),
        warnings,
    })
}

/// 从 pm-tc 任务的最终变量解析出一个 [`Unit`].
///
/// 变量名即 `env:XXX` 去掉前缀后的名称 (例如 `Unit`, `Service.ExecStart`).
///
/// `raw` 为配置文件中该 task 的原始 `env:XXX` 模板, 用于命令字段的
/// [`exec_eval_2`] 解析; `r` 为 pm-tc 的递归深度上限.
fn parse_unit(
    env: &BTreeMap<String, String>,
    raw: &HashMap<String, HasP<String>>,
    r: u16,
    warnings: &mut Vec<String>,
) -> Result<Unit, ConfError> {
    let name = get_req(env, "Unit")?;
    let unit_type = parse_unit_type(&name)?;

    // 对 `Unit.` / `Service.` / `Install.` / `Timer.` / `Win.` 开头且不支持的
    // key 产生警告 (由调用方输出并写入日志); 其余 key 静默忽略.
    for key in unsupported_keys(env) {
        warnings.push(format!(
            "unit `{}`: unsupported key `{}`; ignored",
            name, key
        ));
    }

    let description = get(env, "Unit.Description").unwrap_or("").to_string();
    // 默认值 代码中写死; 用户在配置中指定时读取并解析.
    let start_limit_interval_sec = get_u32(env, "Unit.StartLimitIntervalSec", "10")?;
    let start_limit_burst = get_u32(env, "Unit.StartLimitBurst", "5")?;

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

    // 当前 task 的最终变量集 (供 exec_eval 展开命令模板).
    let tenv = task_env(env);

    let exec_start = {
        let h = raw
            .get("Service.ExecStart")
            .ok_or_else(|| ConfError::BadUnit("missing `Service.ExecStart`".to_string()))?;
        let argv = exec_eval_2(h.as_ref(), &tenv, r).map_err(|e| ConfError::Pm(e.to_string()))?;
        if argv.is_empty() {
            return Err(ConfError::BadUnit(format!(
                "unit `{}`: Service.ExecStart is empty",
                name
            )));
        }
        argv
    };

    let exec_stop = exec_opt(raw.get("Service.ExecStop"), &tenv, r)?;
    let exec_reload = exec_opt(raw.get("Service.ExecReload"), &tenv, r)?;

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
        unit_type,
        description,
        start_limit_interval_sec,
        start_limit_burst,
        service: Service {
            service_type,
            exec_start,
            exec_stop,
            exec_reload,
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

/// 根据 unit 名称后缀解析 [`UnitType`].
///
/// 当前版本 只允许 `service`; `timer` / `target` 属于计划支持但尚未实现的
/// 类型, 给出明确错误.
fn parse_unit_type(name: &str) -> Result<UnitType, ConfError> {
    if name.ends_with(".service") {
        Ok(UnitType::Service)
    } else if name.ends_with(".timer") {
        Err(ConfError::BadUnit(format!(
            "unit `{}`: timer units are not implemented yet",
            name
        )))
    } else if name.ends_with(".target") {
        Err(ConfError::BadUnit(format!(
            "unit `{}`: custom target units are not supported",
            name
        )))
    } else {
        Err(ConfError::BadUnit(format!(
            "unit `{}`: unsupported unit type (supported suffixes: .service, .timer, .target)",
            name
        )))
    }
}

/// 本版本 已支持的 key.
///
/// 其余以 `Unit.`, `Service.`, `Install.`, `Timer.`, `Win.`
/// 开头的 key 视为不支持.
const SUPPORTED_KEYS: &[&str] = &[
    "Unit.Description",
    "Unit.StartLimitIntervalSec",
    "Unit.StartLimitBurst",
    "Service.Type",
    "Service.ExecStart",
    "Service.ExecStop",
    "Service.ExecReload",
    "Service.Restart",
    "Service.RestartSec",
    "Service.TimeoutStartSec",
    "Service.TimeoutStopSec",
    "Service.WorkingDirectory",
    "Service.Environment",
    "Service.RemainAfterExit",
    "Install.WantedBy",
    "Win.CreateNoWindow",
];

/// 返回 `env` 中以 `Unit.`, `Service.`, `Install.`, `Timer.`, `Win.`
/// 开头且不在 [`SUPPORTED_KEYS`] 中的 key (已按 key 排序).
fn unsupported_keys(env: &BTreeMap<String, String>) -> Vec<String> {
    env.keys()
        .filter(|k| {
            k.starts_with("Unit.")
                || k.starts_with("Service.")
                || k.starts_with("Install.")
                || k.starts_with("Timer.")
                || k.starts_with("Win.")
        })
        .filter(|k| !SUPPORTED_KEYS.contains(&k.as_str()))
        .cloned()
        .collect()
}

/// 把当前 task 的最终变量转换为 pm-tc 变量集 (供 [`exec_eval_2`] 使用).
fn task_env(env: &BTreeMap<String, String>) -> PmtcEnv {
    let mut o = PmtcEnv::new();
    for (k, v) in env {
        o.set(k, v);
    }
    o
}

/// 解析可选命令 (缺失或切分后为空返回 `None`).
fn exec_opt(
    h: Option<&HasP<String>>,
    env: &PmtcEnv,
    r: u16,
) -> Result<Option<Vec<String>>, ConfError> {
    let Some(h) = h else {
        return Ok(None);
    };
    let argv = exec_eval_2(h.as_ref(), env, r).map_err(|e| ConfError::Pm(e.to_string()))?;
    Ok(if argv.is_empty() { None } else { Some(argv) })
}

/// 读取变量值.
fn get<'a>(env: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    env.get(key).map(|s| s.as_str())
}

/// 读取必需的变量值 (非空).
fn get_req(env: &BTreeMap<String, String>, key: &str) -> Result<String, ConfError> {
    match get_nonempty(env, key) {
        Some(v) => Ok(v),
        None => Err(ConfError::BadUnit(format!("missing `{}`", key))),
    }
}

/// 读取非空变量值.
fn get_nonempty(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    get(env, key)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// 读取非负整数变量值 (带默认值).
fn get_u32(env: &BTreeMap<String, String>, key: &str, default: &str) -> Result<u32, ConfError> {
    let s = get(env, key).unwrap_or(default);
    s.parse::<u32>().map_err(|_| {
        ConfError::BadUnit(format!("`{}` must be a non-negative integer: `{}`", key, s))
    })
}

/// 读取布尔变量值 (带默认值).
///
/// 支持 systemd 布尔语法:
/// `1`, `yes`, `true`, `on`.
/// `0`, `no`, `false`, `off`.
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

/// 把相对路径解析为相对 `base` 的绝对路径.
fn resolve_path(base: &Path, p: &str) -> String {
    let path = Path::new(p);
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    full.to_string_lossy().into_owned()
}
