//! P1 社交数据与规则：角色间关系、猜拳 PK、串门寒暄、事件日志。
//!
//! 文件：`<data_dir>/social.json`（relations + events）。
//! 规则全部收敛在本模块：
//! - 关系对 a<b 字典序归一；
//! - PK：胜方关系好感 +10，平局双方各 +4；每日每对 3 次防刷；
//! - 寒暄：每次 +1，每日每对 5 次封顶；
//! - 关系 4 级：初识 0-99 / 朋友 100-299 / 闺蜜·搭档 300-599 / 羁绊 600+；
//! - 事件日志上限 200 条环形裁剪。

use crate::settings::data_dir;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use thiserror::Error;

pub const EVENTS_CAP: usize = 200;
pub const PK_DAILY_LIMIT: u32 = 3;
pub const GREET_DAILY_LIMIT: u32 = 5;

#[derive(Debug, Error)]
pub enum SocialStoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("settings error: {0}")]
    Settings(#[from] crate::settings::SettingsError),
}

/// 每日 PK 计数（防刷）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PkCount {
    pub date: String,
    pub count: u32,
}

/// 两个角色之间的关系（无序对，落盘前已归一为 a<b）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Relation {
    pub a: String,
    pub b: String,
    pub affinity: i32,
    pub level: u8,
    pub title: String,
    #[serde(rename = "pk_count")]
    pub pk_count: PkCount,
    #[serde(rename = "greet_today")]
    pub greet_today: u32,
    #[serde(rename = "greet_date")]
    pub greet_date: String,
    #[serde(rename = "summon_count")]
    pub summon_count: u32,
    #[serde(rename = "greet_count")]
    pub greet_count: u32,
    #[serde(rename = "pk_total")]
    pub pk_total: u32,
    #[serde(rename = "last_interaction_ts")]
    pub last_interaction_ts: u64,
}

/// 事件日志条目。type ∈ greet|pk|level_up|summon|dismiss。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialEvent {
    pub ts: u64,
    #[serde(rename = "type")]
    pub kind: String,
    pub a: String,
    pub b: String,
    pub detail: serde_json::Value,
}

/// 桌宠在场状态（双窗口契约 §4.1，持久化于 social.json 新增字段）。
/// `present` 恒含主角色；`locked` 记录每角色位置锁定标记。
/// BTreeMap 保证 locked 键序稳定、序列化输出确定。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeskState {
    pub present: Vec<String>,
    pub locked: std::collections::BTreeMap<String, bool>,
}

impl Default for DeskState {
    fn default() -> Self {
        Self {
            present: Vec::new(),
            locked: std::collections::BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SocialFile {
    pub version: u32,
    pub relations: Vec<Relation>,
    pub events: Vec<SocialEvent>,
    /// 桌宠在场状态（双窗口契约 §4）。旧 social.json 无此字段时取默认值，
    /// 迁移幂等：旧文件直接读入，不重写即无损。
    #[serde(default)]
    pub desk: DeskState,
}

impl Default for SocialFile {
    fn default() -> Self {
        Self {
            version: 1,
            relations: Vec::new(),
            events: Vec::new(),
            desk: DeskState::default(),
        }
    }
}

// ---- 等级表（冻结） ----

pub struct LevelInfo {
    pub level: u8,
    pub name: &'static str,
    pub min: i32,
    pub max: i32,
}

pub const LEVELS: [LevelInfo; 4] = [
    LevelInfo { level: 0, name: "初识", min: 0, max: 99 },
    LevelInfo { level: 1, name: "朋友", min: 100, max: 299 },
    LevelInfo { level: 2, name: "闺蜜/搭档", min: 300, max: 599 },
    LevelInfo { level: 3, name: "羁绊", min: 600, max: i32::MAX },
];

pub fn level_of(affinity: i32) -> u8 {
    match affinity {
        0..=99 => 0,
        100..=299 => 1,
        300..=599 => 2,
        _ => 3,
    }
}

pub fn level_name(level: u8) -> &'static str {
    LEVELS.get(level as usize).map(|l| l.name).unwrap_or("初识")
}

/// 升级解锁彩蛋台词（按到达的新等级）。
pub fn level_up_egg(new_level: u8) -> &'static str {
    match new_level {
        1 => "哇，从今天起我们就是朋友啦！以后常来串门哦～",
        2 => "我们已经这么要好了吗？真好呀，搭档！",
        3 => "能遇见你真好……这份默契，是谁都换不走的。",
        _ => "关系更近了一步。",
    }
}

// ---- 日期工具（本地按 UTC+8 / Asia-Shanghai） ----

fn now_ts() -> u64 {
    crate::storage::now_ts()
}

/// Unix 秒 → (年, 月, 日)，按 UTC+8 偏移。
pub fn today_str() -> String {
    let secs = now_ts() as i64 + 8 * 3600;
    let days = secs.div_euclid(86400);
    // Howard Hinnant civil-from-days
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}")
}

// ---- 猜拳 ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Rock,
    Paper,
    Scissors,
}

impl Move {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rock" => Some(Move::Rock),
            "paper" => Some(Move::Paper),
            "scissors" => Some(Move::Scissors),
            _ => None,
        }
    }
    pub fn as_zh(self) -> &'static str {
        match self {
            Move::Rock => "石头",
            Move::Paper => "布",
            Move::Scissors => "剪刀",
        }
    }
    /// 比较：self 对 other。
    pub fn beats(self, other: Move) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        match (self, other) {
            (a, b) if a == b => Equal,
            (Move::Rock, Move::Scissors)
            | (Move::Scissors, Move::Paper)
            | (Move::Paper, Move::Rock) => Greater,
            _ => Less,
        }
    }
}

// ---- 寒暄模板（按 pair × 关系等级分档，每档 3 条候选） ----

/// 每档 3 条候选：(对 A 说的台词, 对 B 说的台词)。
const GREET_SCRIPTS: [[(&str, &str); 3]; 4] = [
    // level 0 初识
    [
        ("是你呀，初次见面，请多关照～", "你好你好，以后就是邻居咯～"),
        ("今天天气不错呢，你也出来走走吗？", "对呀，正想找谁打个招呼呢。"),
        ("请问……这里有人吗？", "没有人哦，坐吧坐吧，别客气。"),
    ],
    // level 1 朋友
    [
        ("嘿，你来啦！最近忙不忙呀？", "刚忙完，第一个就想来找你玩～"),
        ("上次你说的那家小店我去啦，真不错！", "对吧对吧！下次一起去。"),
        ("今天状态怎么样？", "被你一问就更有精神了！"),
    ],
    // level 2 闺蜜/搭档
    [
        ("宝！今天状态好好呀～", "嘿嘿，就知道你最懂我。"),
        ("走走走，陪我去那边转转！", "走呀走呀，你说去哪就去哪。"),
        ("跟你待在一起最放松了。", "我也是呀，知心搭档～"),
    ],
    // level 3 羁绊
    [
        ("不管今天顺不顺，看到你就安心了。", "我也是。以后的日子我们都一起走。"),
        ("有你在身边，什么难事都不怕。", "嗯，约定好了哦，不许食言。"),
        ("老朋友，又见面啦。", "一直都在呀，我哪都不去。"),
    ],
];

/// 选一条寒暄模板。speaker1=from，speaker2=to；用 (a,b,level) 哈希做种子，
/// 保证同档内候选随角色对与等级变化。
pub fn pick_greet_script(a: &str, b: &str, level: u8) -> (String, String) {
    let mut h = DefaultHasher::new();
    a.hash(&mut h);
    b.hash(&mut h);
    (level as u64).hash(&mut h);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0)
        .hash(&mut h);
    let idx = (h.finish() as usize) % GREET_SCRIPTS[0].len();
    let (l1, l2) = GREET_SCRIPTS[level.clamp(0, 3) as usize][idx];
    (l1.to_string(), l2.to_string())
}

// ---- Store ----

pub struct SocialStore {
    path: std::path::PathBuf,
    pub file: SocialFile,
}

impl Default for SocialStore {
    fn default() -> Self {
        Self::load().unwrap_or_else(|_| Self::fallback())
    }
}

impl SocialStore {
    pub fn load() -> Result<Self, SocialStoreError> {
        let path = data_dir()?.join("social.json");
        let file = if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            SocialFile::default()
        };
        let s = Self { path, file };
        s.persist()?;
        Ok(s)
    }

    fn fallback() -> Self {
        Self {
            path: std::env::temp_dir().join("qibai-social-fallback.json"),
            file: SocialFile::default(),
        }
    }

    pub fn persist(&self) -> Result<(), SocialStoreError> {
        let raw = serde_json::to_string_pretty(&self.file)?;
        // 问题7-2：原子写（tmp+rename）。
        crate::fsutil::atomic_write_json(&self.path, &raw)?;
        Ok(())
    }

    /// 归一为 (a,b)，a<b 字典序。
    pub fn normalize<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
        if a <= b {
            (a, b)
        } else {
            (b, a)
        }
    }

    /// 取（或建）一对关系；同时按今天日期重置每日计数器。
    pub fn relation_mut(&mut self, a: &str, b: &str) -> &mut Relation {
        let (x, y) = Self::normalize(a, b);
        let today = today_str();
        if let Some(pos) = self
            .file
            .relations
            .iter()
            .position(|r| r.a == x && r.b == y)
        {
            let r = &mut self.file.relations[pos];
            if r.pk_count.date != today {
                r.pk_count = PkCount {
                    date: today.clone(),
                    count: 0,
                };
            }
            if r.greet_date != today {
                r.greet_date = today;
                r.greet_today = 0;
            }
            return r;
        }
        self.file.relations.push(Relation {
            a: x.to_string(),
            b: y.to_string(),
            affinity: 0,
            level: 0,
            title: level_name(0).to_string(),
            pk_count: PkCount {
                date: today.clone(),
                count: 0,
            },
            greet_today: 0,
            greet_date: today,
            summon_count: 0,
            greet_count: 0,
            pk_total: 0,
            last_interaction_ts: 0,
        });
        self.file.relations.last_mut().unwrap()
    }

    pub fn relation(&self, a: &str, b: &str) -> Option<&Relation> {
        let (x, y) = Self::normalize(a, b);
        self.file
            .relations
            .iter()
            .find(|r| r.a == x && r.b == y)
    }

    /// 给关系加好感度，重算等级。返回 (旧 level, 新 level)。
    pub fn add_affinity(&mut self, a: &str, b: &str, delta: i32) -> (u8, u8) {
        let r = self.relation_mut(a, b);
        let old = r.level;
        r.affinity = r.affinity.max(0) + delta;
        let new = level_of(r.affinity);
        r.level = new;
        r.title = level_name(new).to_string();
        r.last_interaction_ts = now_ts();
        (old, new)
    }

    /// 追加事件；超过 [`EVENTS_CAP`] 环形裁剪。
    pub fn push_event(&mut self, kind: &str, a: &str, b: &str, detail: serde_json::Value) {
        self.file.events.push(SocialEvent {
            ts: now_ts(),
            kind: kind.to_string(),
            a: a.to_string(),
            b: b.to_string(),
            detail,
        });
        if self.file.events.len() > EVENTS_CAP {
            let drop = self.file.events.len() - EVENTS_CAP;
            self.file.events.drain(0..drop);
        }
    }

    // ---- desk 在场状态簿记（契约 §4） ----

    /// 召唤在场（幂等：已在场则 no-op），落盘。
    pub fn desk_summon(&mut self, id: &str) -> Result<(), SocialStoreError> {
        if !self.file.desk.present.iter().any(|p| p == id) {
            self.file.desk.present.push(id.to_string());
        }
        self.persist()
    }

    /// 送走在场；同时清掉该角色的锁定标记，落盘。
    pub fn desk_dismiss(&mut self, id: &str) -> Result<(), SocialStoreError> {
        self.file.desk.present.retain(|p| p != id);
        self.file.desk.locked.remove(id);
        self.persist()
    }

    /// 写入位置锁定标记，落盘。
    pub fn desk_set_locked(&mut self, id: &str, locked: bool) -> Result<(), SocialStoreError> {
        self.file.desk.locked.insert(id.to_string(), locked);
        self.persist()
    }

    /// 切换主角色时的 desk 簿记：`present = [new] ∪ (present \ {old})`（契约 §4.2）。
    pub fn desk_replace_active(&mut self, old: &str, new: &str) -> Result<(), SocialStoreError> {
        self.file
            .desk
            .present
            .retain(|p| p != old && p != new);
        self.file.desk.present.insert(0, new.to_string());
        self.persist()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> std::path::PathBuf {
        // 调用方需在测试体内持 env_lock。
        let tmp = crate::test_support::unique_temp_dir("social");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        tmp
    }
    fn teardown() {
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn pair_normalized_lexicographically() {
        let (x, y) = SocialStore::normalize("huopo", "gudian");
        assert_eq!((x, y), ("gudian", "huopo"));
    }

    #[test]
    fn level_buckets_are_correct() {
        assert_eq!(level_of(0), 0);
        assert_eq!(level_of(99), 0);
        assert_eq!(level_of(100), 1);
        assert_eq!(level_of(299), 1);
        assert_eq!(level_of(300), 2);
        assert_eq!(level_of(599), 2);
        assert_eq!(level_of(600), 3);
        assert_eq!(level_of(10000), 3);
    }

    #[test]
    fn move_judgement_is_correct() {
        use std::cmp::Ordering::*;
        assert_eq!(Move::Rock.beats(Move::Scissors), Greater);
        assert_eq!(Move::Scissors.beats(Move::Rock), Less);
        assert_eq!(Move::Paper.beats(Move::Rock), Greater);
        assert_eq!(Move::Scissors.beats(Move::Paper), Greater);
        assert_eq!(Move::Rock.beats(Move::Rock), Equal);
        assert_eq!(Move::parse("rock"), Some(Move::Rock));
        assert_eq!(Move::parse("paper"), Some(Move::Paper));
        assert_eq!(Move::parse("scissors"), Some(Move::Scissors));
        assert_eq!(Move::parse("fire"), None);
    }

    #[test]
    fn greet_template_nonempty_for_all_pairs_and_levels() {
        let ids = ["gudian", "dushi", "wenrou", "huopo", "reqing"];
        for &a in &ids {
            for &b in &ids {
                if a == b {
                    continue;
                }
                for level in 0u8..4 {
                    let (l1, l2) = pick_greet_script(a, b, level);
                    assert!(!l1.is_empty(), "empty line1 {a}/{b}/{level}");
                    assert!(!l2.is_empty(), "empty line2 {a}/{b}/{level}");
                }
            }
        }
    }

    #[test]
    fn events_ring_at_200() {
        let _g = crate::test_support::env_lock();
        let tmp = setup();
        let mut s = SocialStore::default();
        for i in 0..250 {
            s.push_event("greet", "gudian", "huopo", serde_json::json!({"i": i}));
        }
        assert_eq!(s.file.events.len(), EVENTS_CAP);
        // 最旧的 50 条（i=0..49）被裁掉
        assert_eq!(s.file.events[0].detail["i"], 50);
        assert_eq!(s.file.events[199].detail["i"], 249);
        let _ = tmp;
        teardown();
    }

    #[test]
    fn daily_counters_reset_on_new_day() {
        let _g = crate::test_support::env_lock();
        let tmp = setup();
        let mut s = SocialStore::default();
        {
            let r = s.relation_mut("gudian", "huopo");
            r.pk_count.count = PK_DAILY_LIMIT;
            r.pk_count.date = "1999-01-01".to_string(); // 昨天
        }
        let r = s.relation_mut("gudian", "huopo");
        assert_eq!(r.pk_count.count, 0); // 跨天自动重置
        assert_eq!(r.pk_count.date, today_str());
        let _ = tmp;
        teardown();
    }

    #[test]
    fn add_affinity_recomputes_level_and_title() {
        let _g = crate::test_support::env_lock();
        let tmp = setup();
        let mut s = SocialStore::default();
        let (old, new) = s.add_affinity("gudian", "huopo", 99);
        assert_eq!((old, new), (0, 0));
        let (old, new) = s.add_affinity("gudian", "huopo", 1);
        assert_eq!((old, new), (0, 1));
        assert_eq!(s.relation("gudian", "huopo").unwrap().title, "朋友");
        let _ = tmp;
        teardown();
    }

    /// 契约 §4.4-1：无 desk 字段的旧 social.json 读入后 desk 为默认值，迁移幂等。
    #[test]
    fn desk_migration_defaults_when_absent() {
        let _g = crate::test_support::env_lock();
        let tmp = setup();
        std::fs::write(
            tmp.join("social.json"),
            r#"{"version":1,"relations":[],"events":[]}"#,
        )
        .unwrap();
        let s = SocialStore::default();
        assert!(s.file.desk.present.is_empty());
        assert!(s.file.desk.locked.is_empty());
        // 重载一次仍是默认值（不报错、不漂移）
        let s2 = SocialStore::default();
        assert!(s2.file.desk.present.is_empty());
        assert!(s2.file.desk.locked.is_empty());
        let _ = tmp;
        teardown();
    }
}
