//! UnitS: unit 配置与 mask/enable 持久状态的内存数据库.
//!
//! UnitS 只持有数据与 读取/更新 方法, 不做任何 IO. 由 UnitD (通过 [`super::actor::UnitDSM`]
//! 状态机) 单线程访问, 无需内部加锁.

use std::collections::{HashMap, HashSet};

use crate::conf::Unit;

use super::query::UnitInfo;

/// 单个 unit 的内存配置条目 (静态配置, 只读)
#[derive(Clone)]
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

    /// `Install.WantedBy` (决定该 unit 是否可 enable)
    fn wanted_by(&self) -> Option<&str> {
        self.conf.install.wanted_by.as_deref()
    }
}

/// unit 配置与持久状态的内存数据库
#[derive(Clone)]
pub struct UnitS {
    /// 当前配置
    current: HashMap<String, UnitEntry>,
    /// 已 mask / enable 的 unit 名称 (持久状态缓存)
    me: UnitSME,
}

/// 已 mask / enabled 的 unit 名称 (持久状态缓存)
#[derive(Clone, Default)]
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

        let mut masked: HashSet<String> = me.masked.into_iter().collect();
        let mut enabled: HashSet<String> = me.enabled.into_iter().collect();
        // 清理已不存在 unit 的持久状态缓存
        masked.retain(|n| current.contains_key(n));
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

    /// 读取 unit 是否 enable
    pub fn enabled(&self, name: &str) -> bool {
        self.me.enabled.contains(name)
    }

    /// 读取 unit 是否 mask
    pub fn masked(&self, name: &str) -> bool {
        self.me.masked.contains(name)
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

    /// 读取一个 unit 的查询快照 (不存在则 None)
    pub fn info_of(&self, name: &str) -> Option<UnitInfo> {
        let c = self.get(name)?;
        Some(UnitInfo {
            name: c.name.clone(),
            description: c.description.clone(),
            unit_type: c.unit_type,
            enabled: self.enabled(name),
            masked: self.masked(name),
        })
    }

    /// 读取全部 unit 的查询快照 (按名称排序)
    pub fn all_infos(&self) -> Vec<UnitInfo> {
        self.all_names()
            .iter()
            .map(|n| self.info_of(n).expect("unit exists"))
            .collect()
    }

    /// 用于 daemon-reload 重建
    pub fn get_me(&self) -> UnitSME {
        self.me.clone()
    }

    /// 应该随指定 target 启动的 unit 名称 (已 enable 且未 mask, `WantedBy` 匹配)
    pub fn target_units(&self, target: &str) -> Vec<String> {
        let mut out = Vec::new();
        for name in self.current.keys() {
            if self.enabled(name) && !self.masked(name) {
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
