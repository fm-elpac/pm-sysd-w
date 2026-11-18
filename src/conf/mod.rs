//! 配置文件解析/加载.
//!
//! pm-sysd-w 使用 pm-tc 的集中式 toml 配置, 把多个 unit 写在一个文件中. 本模块
//! 调用 pm-tc 的 `task::plan("sysd")`, 从 `PlanDebug` 中取出所有 unit 配置.
//!
//! 配置格式参考 `doc/user/config.md`.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    error::Error,
    fs,
    io::Read,
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

/// 所有 toml 文件总大小上限 (字节)
pub const MAX_CONF_TOTAL_BYTE: usize = 10 * 1024 * 1024;
/// unit 名称最大长度 (utf-8 字节)
pub const MAX_UNIT_NAME_BYTE: usize = 500;
/// 单个受支持 `env:` 键最大长度 (utf-8 字节)
pub const MAX_UNIT_KEY_BYTE: usize = 500;
/// 单个受支持 `env:` 值最大长度 (utf-8 字节)
pub const MAX_UNIT_VALUE_BYTE: usize = 20 * 1024;
/// 单个 unit 的全部受支持键值总长上限 (utf-8 字节)
pub const MAX_UNIT_TOTAL_BYTE: usize = 500 * 1024;

/// 已支持的 key.
///
/// 其余以 `Unit.`, `Service.`, `Install.`, `Timer.`, `Win.`
/// 开头的 key 视为不支持.
pub const SUPPORTED_KEYS: &[&str] = &[
    "Unit.Description",
    "Unit.StartLimitIntervalSec",
    "Unit.StartLimitBurst",
    "Service.Type",
    "Service.RemainAfterExit",
    "Service.ExecStart",
    "Service.ExecStop",
    "Service.ExecReload",
    "Service.Restart",
    "Service.RestartSec",
    "Service.TimeoutStartSec",
    "Service.TimeoutStopSec",
    "Service.WorkingDirectory",
    "Service.Environment",
    "Install.WantedBy",
    "Win.CreateNoWindow",
];

/// 支持 `多行语法` 的字段前缀
pub const MULTI_LINE_PREFIXES: &[&str] = &[
    "Service.ExecStart",
    "Service.ExecStop",
    "Service.ExecReload",
    "Service.Environment",
    "Install.WantedBy",
];

/// 判断一个 key 是否受支持 (含 `多行语法` 变体).
fn is_supported_key(key: &str) -> bool {
    if SUPPORTED_KEYS.contains(&key) {
        return true;
    }
    MULTI_LINE_PREFIXES.iter().any(|p| {
        key.strip_prefix(p)
            .is_some_and(|suffix| suffix.starts_with('.'))
    })
}

/// 读取配置文件的默认实现
struct FsReader {
    /// 已读取的 toml 文件总字节数
    total: usize,
    /// 所有 toml 文件总大小上限 (字节)
    limit: usize,
}

impl ReadConfFile for FsReader {
    fn read(&mut self, path: &str) -> Result<String, Box<dyn Error>> {
        // 只读取剩余额度 + 1 字节, 用于在超限时立即失败, 避免把超大文件
        // 整个读入内存.
        let remaining = self.limit - self.total;
        let mut buf = Vec::with_capacity(remaining + 1);
        fs::File::open(path)?
            .take((remaining + 1) as u64)
            .read_to_end(&mut buf)?;

        if buf.len() > remaining {
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("total size of config files exceeds {} bytes", self.limit),
            )));
        }

        let text = String::from_utf8(buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        self.total += text.len();
        Ok(text)
    }
}

/// 检查环境现状的默认实现
struct FsChecker;

impl CheckEnv for FsChecker {
    fn file_exist(&mut self, path: &str) -> Result<bool, Box<dyn Error>> {
        Ok(Path::new(path).exists())
    }

    fn file_read(&mut self, _path: &str) -> Result<String, Box<dyn Error>> {
        // pm-sysd-w 不使用 pm-tc 的文件读取能力.
        // 始终报错, 避免意外读取超大文件.
        Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "file_read is not supported",
        )))
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

    // 加载配置文件
    let mut src = SrcInfo::new();
    let mut rc = FsReader {
        total: 0,
        limit: MAX_CONF_TOTAL_BYTE,
    };
    let c = load(conf_path, &mut rc, &mut src).map_err(|e| ConfError::Pm(e.to_string()))?;

    // 计划执行 (拿到所有 `[[t.sysd]]` 实例的最终变量).
    // 空配置 (没有任何 `[[t.sysd]]`) 也允许加载: 此时传空 task 列表.
    let sysd_tasks = c.task.get("sysd").map(|v| v.as_slice()).unwrap_or(&[]);
    let task: Vec<String> = if sysd_tasks.is_empty() {
        Vec::new()
    } else {
        vec!["sysd".to_string()]
    };
    let mut ce = FsChecker;
    let (_, pd) = plan(&task, &c, &env, &mut ce).map_err(|e| ConfError::Pm(e.to_string()))?;

    // 数据库路径 (`[env] SYSD_DB`)
    let db_rel = pd
        .env0
        .get("SYSD_DB")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "pm_sysd_w_v1.db".to_string());
    let db_path = resolve_path(&conf_dir, &db_rel);

    // 解析每个 unit (命令字段使用 pm-tc exec_eval 切分)
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

    // 检测重复 unit 名称 (名称重复视为配置错误)
    let mut seen = HashSet::new();
    for u in &units {
        if !seen.insert(u.name.as_str()) {
            return Err(ConfError::BadUnit(format!("duplicate unit `{}`", u.name)));
        }
    }

    Ok(Conf {
        units,
        db_path,
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
    check_unit_name(&name)?;
    let unit_type = parse_unit_type(&name)?;
    check_unit_size(env)?;

    // 对 `Unit.` / `Service.` / `Install.` / `Timer.` / `Win.`
    // 开头且不支持的 key 产生警告 (由调用方输出并写入日志), 其余 key 静默忽略.
    for key in unsupported_keys(env) {
        warnings.push(format!(
            "unit `{}`: unsupported key `{}`; ignored",
            name, key
        ));
    }

    let description = get(env, "Unit.Description").unwrap_or("").to_string();
    // 默认值代码中写死, 用户在配置中指定时读取并解析
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

    // 当前 task 的最终变量集 (供 exec_eval 展开命令模板)
    let tenv = task_env(env);

    let remain_after_exit = get_bool(env, "Service.RemainAfterExit", false)?;

    let exec_start = parse_exec_lines(raw, "Service.ExecStart", &name, &tenv, r)?;
    let exec_stop = parse_exec_lines(raw, "Service.ExecStop", &name, &tenv, r)?;
    let exec_reload = parse_exec_lines(raw, "Service.ExecReload", &name, &tenv, r)?;

    check_exec_required(
        &name,
        service_type,
        &exec_start,
        &exec_stop,
        remain_after_exit,
    )?;

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

    let environment = parse_environment_multi(env, raw, &name)?;
    let wanted_by = parse_wanted_by(env, raw, &name, warnings)?;

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
            environment,
            remain_after_exit,
        },
        install: Install { wanted_by },
        win,
    })
}

/// 检查 unit 名称是否符合规则.
///
/// - 通过 pm-tc 的变量名检查 (`pm_tc::te::check_name`).
/// - 最大长度 [`MAX_UNIT_NAME_BYTE`] 字节 (utf-8).
/// - 必须以受支持的后缀 (`.service`, `.timer`, `.target`) 结尾.
/// - 后缀前面不能为空.
fn check_unit_name(name: &str) -> Result<(), ConfError> {
    pm_tc::te::check_name(name, 0)
        .map_err(|e| ConfError::BadUnit(format!("invalid unit name `{}`: {}", name, e)))?;

    if name.len() > MAX_UNIT_NAME_BYTE {
        return Err(ConfError::BadUnit(format!(
            "unit name `{}` exceeds {} bytes",
            name, MAX_UNIT_NAME_BYTE
        )));
    }

    let suffix = [".service", ".timer", ".target"]
        .into_iter()
        .find(|s| name.ends_with(*s));
    let Some(suffix) = suffix else {
        return Err(ConfError::BadUnit(format!(
            "unit `{}`: unsupported unit type (supported suffixes: .service, .timer, .target)",
            name
        )));
    };

    if name.len() == suffix.len() {
        return Err(ConfError::BadUnit(format!(
            "unit name `{}`: name must not be empty before `{}`",
            name, suffix
        )));
    }

    Ok(())
}

/// 根据 unit 名称后缀解析 [`UnitType`].
///
/// 当前版本只允许 `service`.
/// `timer` / `target` 属于计划支持但尚未实现的类型, 给出明确错误.
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

/// 检查 unit 的受支持 key-value 数据大小.
///
/// 只统计 [`SUPPORTED_KEYS`] 中的 key, 其余 key 不参与统计.
///
/// 单个受支持的键值对: 键最长 [`MAX_UNIT_KEY_BYTE`] 字节,
/// 值最长 [`MAX_UNIT_VALUE_BYTE`] 字节.
/// 全部受支持键值总长不得超过 [`MAX_UNIT_TOTAL_BYTE`] 字节.
fn check_unit_size(env: &BTreeMap<String, String>) -> Result<(), ConfError> {
    let mut total = 0usize;
    for key in env.keys() {
        if !is_supported_key(key) {
            continue;
        }
        let value = env.get(key).expect("key from env must be present");
        if key.len() > MAX_UNIT_KEY_BYTE {
            return Err(ConfError::BadUnit(format!(
                "`{}` key ({} bytes) exceeds {} bytes",
                key,
                key.len(),
                MAX_UNIT_KEY_BYTE
            )));
        }
        if value.len() > MAX_UNIT_VALUE_BYTE {
            return Err(ConfError::BadUnit(format!(
                "`{}` value ({} bytes) exceeds {} bytes",
                key,
                value.len(),
                MAX_UNIT_VALUE_BYTE
            )));
        }
        total += key.len() + value.len();
    }
    if total > MAX_UNIT_TOTAL_BYTE {
        return Err(ConfError::BadUnit(format!(
            "unit config total size ({} bytes) exceeds {} bytes",
            total, MAX_UNIT_TOTAL_BYTE
        )));
    }
    Ok(())
}

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
        .filter(|k| !is_supported_key(k))
        .cloned()
        .collect()
}

/// 把当前 task 的最终变量转换为 pm-tc 变量集 (供 [`exec_eval_2`] 使用)
fn task_env(env: &BTreeMap<String, String>) -> PmtcEnv {
    let mut o = PmtcEnv::new();
    for (k, v) in env {
        o.set(k, v);
    }
    o
}

/// 检查 `ExecStart` 和 `ExecStop` 的必填规则 (见 `doc/user/config.md`)
fn check_exec_required(
    name: &str,
    service_type: ServiceType,
    exec_start: &[Vec<String>],
    exec_stop: &[Vec<String>],
    remain_after_exit: bool,
) -> Result<(), ConfError> {
    if exec_start.is_empty() && exec_stop.is_empty() {
        return Err(ConfError::BadUnit(format!(
            "unit `{}`: ExecStart and ExecStop must not both be empty",
            name
        )));
    }
    if service_type == ServiceType::Simple && exec_start.is_empty() {
        return Err(ConfError::BadUnit(format!(
            "unit `{}`: Type=simple requires ExecStart",
            name
        )));
    }
    if exec_start.is_empty() && !remain_after_exit {
        return Err(ConfError::BadUnit(format!(
            "unit `{}`: missing ExecStart requires RemainAfterExit=yes",
            name
        )));
    }
    Ok(())
}

/// 收集某个 `多行语法` 字段的所有 key (按后缀字符串升序排序).
///
/// 返回 `(key, suffix)` 列表. `suffix` 为空表示基础 key (例如
/// `Service.ExecStart`), 非空表示带后缀的 key (例如 `Service.ExecStart.1`).
fn multi_line_keys<'a>(
    raw: &'a HashMap<String, HasP<String>>,
    prefix: &str,
    name: &str,
) -> Result<Vec<(&'a str, &'a str)>, ConfError> {
    let mut out = Vec::new();
    for k in raw.keys() {
        let Some(rest) = k.strip_prefix(prefix) else {
            continue;
        };
        let suffix = if rest.is_empty() {
            ""
        } else if let Some(suffix) = rest.strip_prefix('.') {
            if suffix.is_empty() {
                return Err(ConfError::BadUnit(format!(
                    "unit `{}`: invalid multi-line key `{}` (empty suffix)",
                    name, k
                )));
            }
            suffix
        } else {
            continue;
        };
        out.push((k.as_str(), suffix));
    }
    out.sort_by(|a, b| a.1.cmp(b.1));
    Ok(out)
}

/// 解析 `多行语法` 命令字段 (ExecStart / ExecStop / ExecReload).
///
/// 每个 key 通过 [`exec_eval_2`] 切分为一条命令.
/// 切分后为空的条目表示清空之前已解析的所有命令 (相当于 systemd 的 `ExecStart=` 空行),
/// 后续命令继续追加.
fn parse_exec_lines(
    raw: &HashMap<String, HasP<String>>,
    prefix: &str,
    name: &str,
    env: &PmtcEnv,
    r: u16,
) -> Result<Vec<Vec<String>>, ConfError> {
    let mut out = Vec::new();
    for (key, _) in multi_line_keys(raw, prefix, name)? {
        let h = raw.get(key).expect("key from multi_line_keys must exist");
        let argv = exec_eval_2(h.as_ref(), env, r).map_err(|e| ConfError::Pm(e.to_string()))?;
        if argv.is_empty() {
            out.clear();
        } else {
            out.push(argv);
        }
    }
    Ok(out)
}

/// 解析 `多行语法` 环境变量字段.
///
/// 每个 key 解析一个 `KEY=VALUE`.
/// 值为空的条目表示清空之前已解析的所有环境变量 (相当于 systemd 的 `Environment=` 空行),
/// 后续条目继续追加.
fn parse_environment_multi(
    env: &BTreeMap<String, String>,
    raw: &HashMap<String, HasP<String>>,
    name: &str,
) -> Result<Vec<(String, String)>, ConfError> {
    let mut out = Vec::new();
    for (key, _) in multi_line_keys(raw, "Service.Environment", name)? {
        let Some(value) = get(env, key) else {
            continue;
        };
        if value.is_empty() {
            out.clear();
        } else {
            out.extend(parse_environment(Some(value))?);
        }
    }
    Ok(out)
}

/// 解析 `多行语法` `Install.WantedBy` 字段, 并去除重复值.
///
/// 重复值会产生警告 (由调用方输出并写入日志), 保留首次出现的值.
fn parse_wanted_by(
    env: &BTreeMap<String, String>,
    raw: &HashMap<String, HasP<String>>,
    name: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<String>, ConfError> {
    let mut out = Vec::new();
    for (key, _) in multi_line_keys(raw, "Install.WantedBy", name)? {
        let Some(value) = get_nonempty(env, key) else {
            continue;
        };
        if out.contains(&value) {
            warnings.push(format!(
                "unit `{}`: duplicate Install.WantedBy `{}`, removed",
                name, value
            ));
        } else {
            out.push(value);
        }
    }
    Ok(out)
}

/// 读取变量值
fn get<'a>(env: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    env.get(key).map(|s| s.as_str())
}

/// 读取必需的变量值 (非空)
fn get_req(env: &BTreeMap<String, String>, key: &str) -> Result<String, ConfError> {
    match get_nonempty(env, key) {
        Some(v) => Ok(v),
        None => Err(ConfError::BadUnit(format!("missing `{}`", key))),
    }
}

/// 读取非空变量值
fn get_nonempty(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    get(env, key)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// 读取非负整数变量值 (带默认值)
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

/// 解析 `KEY=VALUE` 环境变量 (单个字符串)
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

/// 把相对路径解析为相对 `base` 的绝对路径
fn resolve_path(base: &Path, p: &str) -> String {
    let path = Path::new(p);
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    full.to_string_lossy().into_owned()
}
