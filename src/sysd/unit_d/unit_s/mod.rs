//! UnitS: unit 配置与 mask/enable 持久状态的内存数据库.
//!
//! UnitS 只持有数据与 读取/更新 方法, 不做任何 IO. 由 UnitD (通过 [`super::sm::Sm`]
//! 状态机) 单线程访问, 无需内部加锁.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::{
    conf::{Unit, UnitType},
    sysu::{EnableSta, LoadSta},
};

use super::query::UnitInfo;

/// 根据 unit 名称后缀推断类型 (配置中已消失的 unit)
pub fn unit_type_by_name(name: &str) -> Option<UnitType> {
    if name.ends_with(".service") {
        Some(UnitType::Service)
    } else if name.ends_with(".timer") {
        Some(UnitType::Timer)
    } else if name.ends_with(".target") {
        Some(UnitType::Target)
    } else {
        None
    }
}

/// 单个 unit 的内存配置条目 (静态配置, 只读)
#[derive(Debug, Clone, Serialize)]
struct UnitEntry {
    /// 静态配置 (只读)
    conf: Unit,
}

impl UnitEntry {
    fn new(conf: Unit) -> Self {
        Self { conf }
    }

    /// 读取静态配置
    fn conf(&self) -> &Unit {
        &self.conf
    }

    /// `Install.WantedBy` (决定该 unit 是否可 enable).
    ///
    /// TODO: sysd 暂只支持一个 WantedBy, 多行语法只取第一个.
    fn wanted_by(&self) -> Option<&str> {
        self.conf.install.wanted_by.first().map(|s| s.as_str())
    }
}

/// unit 配置与持久状态的内存数据库
#[derive(Debug, Clone, Serialize)]
pub struct UnitS {
    /// 当前配置
    current: HashMap<String, UnitEntry>,
    /// 已 mask / enable 的 unit 名称 (持久状态缓存)
    me: UnitSME,
}

/// 已 mask / enabled 的 unit 名称 (持久状态缓存)
#[derive(Debug, Clone, Default, Serialize)]
pub struct UnitSME {
    /// 已 mask 的 unit 名称 (持久状态缓存)
    pub masked: HashSet<String>,
    /// 已 enable 的 unit 名称 (持久状态缓存)
    pub enabled: HashSet<String>,
}

impl UnitS {
    /// 用 unit 配置与 mask/enable 列表创建
    pub fn new(units: Vec<Unit>, me: UnitSME) -> Self {
        let current: HashMap<String, UnitEntry> = units
            .into_iter()
            .map(|u| {
                let name = u.name.clone();
                (name, UnitEntry::new(u))
            })
            .collect();

        let masked: HashSet<String> = me.masked.into_iter().collect();
        let mut enabled: HashSet<String> = me.enabled.into_iter().collect();
        // enable 状态只对当前配置中的 unit 有效; mask 状态保留, 因为被 mask 的
        // unit 即使配置中不存在也仍然视为不存在 (但 mask 状态本身需要保留).
        enabled.retain(|n| current.contains_key(n));

        Self {
            current,
            me: UnitSME { masked, enabled },
        }
    }

    /// 读取一个 unit 的静态配置
    pub fn get(&self, name: &str) -> Option<&Unit> {
        self.current.get(name).map(|e| e.conf())
    }

    /// 判断 unit 是否存在
    pub fn contains(&self, name: &str) -> bool {
        self.current.contains_key(name)
    }

    /// 所有 unit 名称 (排序)
    pub fn all_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.current.keys().cloned().collect();
        names.sort();
        names
    }

    /// 已 mask 的 unit 名称 (排序)
    pub fn masked_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.me.masked.iter().cloned().collect();
        names.sort();
        names
    }

    /// 读取 unit 是否 enable
    pub fn enabled(&self, name: &str) -> bool {
        self.me.enabled.contains(name)
    }

    /// 读取 unit 是否 mask
    pub fn masked(&self, name: &str) -> bool {
        self.me.masked.contains(name)
    }

    /// 读取 unit 的加载状态.
    ///
    /// 被 mask 的 unit 即使配置中不存在也返回 `Masked`; 其它情况返回 `Loaded`.
    pub fn load_sta(&self, name: &str) -> LoadSta {
        if self.masked(name) {
            LoadSta::Masked
        } else {
            LoadSta::Loaded
        }
    }

    /// 读取 unit 的启用状态 (只用于当前配置中存在的 unit)
    pub fn enable_sta(&self, name: &str) -> EnableSta {
        if self.masked(name) {
            EnableSta::Masked
        } else if self.enabled(name) {
            EnableSta::Enabled
        } else if self
            .get(name)
            .and_then(|u| u.install.wanted_by.first())
            .is_some()
        {
            EnableSta::Disabled
        } else {
            EnableSta::Static
        }
    }

    /// 设置 enable 状态 (仅更新内存缓存, 持久化由 UnitD 负责)
    pub fn set_enabled(&mut self, name: &str, value: bool) {
        if value {
            self.me.enabled.insert(name.to_string());
        } else {
            self.me.enabled.remove(name);
        }
    }

    /// 设置 mask 状态 (仅更新内存缓存, 持久化由 UnitD 负责)
    pub fn set_masked(&mut self, name: &str, value: bool) {
        if value {
            self.me.masked.insert(name.to_string());
        } else {
            self.me.masked.remove(name);
        }
    }

    /// 读取一个 unit 的查询快照 (不存在且未 mask 则 None).
    ///
    /// 被 mask 的 unit 无论是否在配置中, 都返回一个仅表示 mask 状态的快照.
    pub fn info_of(&self, name: &str) -> Option<UnitInfo> {
        if self.masked(name) {
            return Some(UnitInfo {
                name: name.to_string(),
                description: String::new(),
                unit_type: unit_type_by_name(name).unwrap_or(UnitType::Service),
                load: LoadSta::Masked,
                enable: EnableSta::Masked,
            });
        }
        let c = self.get(name)?;
        Some(UnitInfo {
            name: c.name.clone(),
            description: c.description.clone(),
            unit_type: c.unit_type,
            load: self.load_sta(name),
            enable: self.enable_sta(name),
        })
    }

    /// 读取全部 unit 的查询快照 (按名称排序, 不包含被 mask 的 unit)
    pub fn all_infos(&self) -> Vec<UnitInfo> {
        self.all_names()
            .into_iter()
            .filter(|n| !self.masked(n))
            .map(|n| self.info_of(&n).expect("unit exists"))
            .collect()
    }

    /// 应该随指定 target 启动的 unit 名称 (已 enable 且未 mask, `WantedBy` 匹配)
    pub fn target_units(&self, target: &str) -> Vec<String> {
        let mut out = Vec::new();
        for name in self.current.keys() {
            if self.enable_sta(name) == EnableSta::Enabled {
                if let Some(w) = self.current.get(name).and_then(|e| e.wanted_by()) {
                    if w == target {
                        out.push(name.clone());
                    }
                }
            }
        }
        out.sort();
        out
    }
}
