//! 亲密度成长系统（L1–L10，冻结规格 docs/INTIMACY_SYSTEM.md）。
//!
//! 全部规则收敛在本模块：
//! - L1–L10 等级表 / `level_of` / 21 个门控键；
//! - 成长来源（chat / click / task / companion / gift / pk_win）与每日总上限 60；
//! - 每日任务（greeting / click / pk）与 9 个成就；
//! - 存储 `<data_dir>/intimacy.json`（entries 按 BTreeMap 有序落盘 + 幂等迁移）。
//!
//! 与 social.json（角色间好感度）完全分离。升级事件由 engine 复用
//! `SocialStore::push_event` 落事件日志（本模块不直接写 social.json）。
//! 隐私本地优先：只落本地，不上传；`sync` 字段只写不读。

use crate::persona::PERSONALITIES;
use crate::personas::SyncState;
use crate::settings::data_dir;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// 每角色每日总积分获取上限（§3.3）。
pub const DAILY_POINTS_CAP: i64 = 60;
/// 每角色每日 click 加分次数上限（§3.1）。
pub const CLICK_DAILY_LIMIT: u32 = 10;
/// 每角色每日 companion 积分上限（§3.2，勘正：9 档阈值 × 1 分 = 9 分打满）。
pub const COMPANION_DAILY_CAP: u32 = 9;
/// companion 累计分钟阈值：每跨过一个 +1（§3.2 冻结公式）。
const COMPANION_THRESHOLDS: [u32; 9] = [10, 20, 30, 50, 70, 90, 120, 150, 180];

#[derive(Debug, Error)]
pub enum IntimacyStoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("settings error: {0}")]
    Settings(#[from] crate::settings::SettingsError),
    #[error("unknown persona: {0}")]
    UnknownPersona(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

// ---- 等级表（冻结 §2） ----

pub struct LevelInfo {
    pub level: u8,
    pub name: &'static str,
    pub min: i64,
    /// None 表示 ∞（L10）。
    pub max: Option<i64>,
    pub desc: &'static str,
}

pub const LEVELS: [LevelInfo; 10] = [
    LevelInfo { level: 1, name: "初识", min: 0, max: Some(120), desc: "初见倾心，从一句你好开始。" },
    LevelInfo { level: 2, name: "熟悉", min: 120, max: Some(280), desc: "开始习惯彼此的存在。" },
    LevelInfo { level: 3, name: "习惯", min: 280, max: Some(480), desc: "你的作息与喜好，她悄悄记下。" },
    LevelInfo { level: 4, name: "贴心", min: 480, max: Some(720), desc: "她开始帮你打理生活小事。" },
    LevelInfo { level: 5, name: "默契", min: 720, max: Some(1000), desc: "一个眼神，她就懂你的心情。" },
    LevelInfo { level: 6, name: "亲近", min: 1000, max: Some(1330), desc: "想为你准备更多惊喜。" },
    LevelInfo { level: 7, name: "羁绊", min: 1330, max: Some(1720), desc: "值得纪念的日子，她不会忘。" },
    LevelInfo { level: 8, name: "依赖", min: 1720, max: Some(2180), desc: "她总想多陪你一会儿。" },
    LevelInfo { level: 9, name: "灵魂", min: 2180, max: Some(2720), desc: "你们之间有了只有彼此懂的默契。" },
    LevelInfo { level: 10, name: "缔结", min: 2720, max: None, desc: "独一无二的约定，只属于你们。" },
];

/// 按阈值取最大满足档（1–10）。
pub fn level_of(points: i64) -> u8 {
    let mut l = 1u8;
    for info in &LEVELS {
        if points >= info.min {
            l = info.level;
        }
    }
    l
}

pub fn level_name(level: u8) -> &'static str {
    LEVELS
        .get(((level - 1) as usize).min(9))
        .map(|l| l.name)
        .unwrap_or("初识")
}

// ---- 门控表（冻结 §4，21 键） ----

pub struct GateInfo {
    pub gate: &'static str,
    pub required_level: u8,
    pub label: &'static str,
    /// "active" | "placeholder"（placeholder 仅影响 UI 展示，不影响判定）。
    pub kind: &'static str,
}

pub const GATES: [GateInfo; 21] = [
    GateInfo { gate: "basic_chat", required_level: 1, label: "基础对话", kind: "active" },
    GateInfo { gate: "idle_actions", required_level: 1, label: "待机动作", kind: "active" },
    GateInfo { gate: "sit_reminder", required_level: 1, label: "久坐提醒", kind: "active" },
    GateInfo { gate: "custom_nickname", required_level: 2, label: "自定义称呼", kind: "active" },
    GateInfo { gate: "daily_greeting", required_level: 2, label: "每日问候", kind: "active" },
    GateInfo { gate: "water_reminder", required_level: 2, label: "喝水提醒", kind: "active" },
    GateInfo { gate: "preference_memory", required_level: 3, label: "记住偏好（作息/常用软件）", kind: "active" },
    GateInfo { gate: "boot_greeting", required_level: 3, label: "开机问候", kind: "active" },
    GateInfo { gate: "assistant_skills", required_level: 4, label: "助理技能（打开应用/文件夹、设提醒）", kind: "active" },
    GateInfo { gate: "vault", required_level: 4, label: "本地密码保险箱", kind: "active" },
    GateInfo { gate: "mood_empathy", required_level: 5, label: "心情识别增强", kind: "active" },
    GateInfo { gate: "screen_guard", required_level: 5, label: "全屏守护", kind: "placeholder" },
    GateInfo { gate: "performance", required_level: 6, label: "表演系统（节日彩蛋接口）", kind: "active" },
    GateInfo { gate: "dressup", required_level: 6, label: "形象换装", kind: "placeholder" },
    GateInfo { gate: "storyline_1", required_level: 7, label: "专属剧情线一（对话彩蛋模板）", kind: "active" },
    GateInfo { gate: "anniversary", required_level: 7, label: "纪念日自动庆祝（纪念日设置）", kind: "active" },
    GateInfo { gate: "sleep_guard", required_level: 8, label: "息屏守护", kind: "placeholder" },
    GateInfo { gate: "voice_mode", required_level: 8, label: "语音陪伴模式", kind: "placeholder" },
    GateInfo { gate: "storyline_2", required_level: 9, label: "专属剧情线二（双人彩蛋）", kind: "active" },
    GateInfo { gate: "ceremony", required_level: 10, label: "缔结仪式结局演出（文字+CSS 动效）", kind: "active" },
    GateInfo { gate: "batch_skills", required_level: 10, label: "高级助理批量技能", kind: "placeholder" },
];

pub fn gate_info(gate: &str) -> Option<&'static GateInfo> {
    GATES.iter().find(|g| g.gate == gate)
}

pub fn gate_required(gate: &str) -> Option<u8> {
    gate_info(gate).map(|g| g.required_level)
}

/// `is_gate_open(level, gate) = level >= gate_required(gate)`；未知键返回 false。
pub fn is_gate_open(level: u8, gate: &str) -> bool {
    gate_required(gate).map(|req| level >= req).unwrap_or(false)
}

// ---- 每日任务 / 成就（冻结 §3.4 / §5） ----

pub const TASKS: [(&str, &str); 3] = [
    ("greeting", "和她说早安"),
    ("click", "完成一次互动"),
    ("pk", "赢一场猜拳"),
];

pub const ACHIEVEMENTS: [(&str, &str); 9] = [
    ("first_greeting", "初次问候"),
    ("chat_100", "聊得来"),
    ("click_50", "指尖温情"),
    ("pk_win_5", "猜拳小能手"),
    ("companion_7", "常伴七日"),
    ("level_3", "心有默契"),
    ("level_6", "亲密无间"),
    ("level_9", "灵魂共鸣"),
    ("level_10", "缔结之约"),
];

pub fn achievement_name(id: &str) -> Option<&'static str> {
    ACHIEVEMENTS.iter().find(|(aid, _)| *aid == id).map(|(_, name)| *name)
}

// ---- 存储结构（冻结 §7） ----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Anniversary {
    pub month: u8,
    pub day: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TaskFlags {
    #[serde(default)]
    pub greeting: bool,
    #[serde(default)]
    pub click: bool,
    #[serde(default)]
    pub pk: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DailyState {
    pub date: String,
    pub gained: i64,
    #[serde(default)]
    pub chats: u32,
    #[serde(default)]
    pub clicks: u32,
    #[serde(default)]
    pub companion_minutes: u32,
    #[serde(default)]
    pub companion_points: u32,
    pub tasks: TaskFlags,
    /// 当日 companion>0 是否已记过 companion_days（防重复计数）。
    #[serde(default)]
    pub companion_day_marked: bool,
}

impl Default for DailyState {
    fn default() -> Self {
        Self {
            date: String::new(),
            gained: 0,
            chats: 0,
            clicks: 0,
            companion_minutes: 0,
            companion_points: 0,
            tasks: TaskFlags::default(),
            companion_day_marked: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CumulativeState {
    #[serde(default)]
    pub chats: u32,
    #[serde(default)]
    pub clicks: u32,
    #[serde(default)]
    pub pk_wins: u32,
    #[serde(default)]
    pub companion_days: u32,
    #[serde(default)]
    pub greetings: u32,
    #[serde(default)]
    pub gifts: u32,
    #[serde(default)]
    pub last_gift_date: Option<String>,
}

impl Default for CumulativeState {
    fn default() -> Self {
        Self {
            chats: 0,
            clicks: 0,
            pk_wins: 0,
            companion_days: 0,
            greetings: 0,
            gifts: 0,
            last_gift_date: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AchievementRecord {
    pub id: String,
    pub ts: u64,
}

/// 单角色亲密度实体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntimacyEntry {
    pub persona_id: String,
    pub points: i64,
    pub level: u8,
    /// 缺字段旧文件（§7.4）serde default 补齐，不报错不丢数据。
    #[serde(default)]
    pub daily: DailyState,
    #[serde(default)]
    pub cumulative: CumulativeState,
    #[serde(default)]
    pub achievements: Vec<AchievementRecord>,
    #[serde(default)]
    pub anniversary: Option<Anniversary>,
    #[serde(default)]
    pub sync: SyncState,
}

impl IntimacyEntry {
    fn new(id: &str) -> Self {
        Self {
            persona_id: id.to_string(),
            points: 0,
            level: 1,
            daily: DailyState::default(),
            cumulative: CumulativeState::default(),
            achievements: Vec::new(),
            anniversary: None,
            sync: SyncState::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntimacyFile {
    pub version: u32,
    /// 内部迁移标志：personas.json 的 intimacy 是否已播种为积分起点。
    #[serde(default)]
    pub migrated: bool,
    /// BTreeMap 保证键序稳定、序列化输出确定。
    pub entries: BTreeMap<String, IntimacyEntry>,
}

impl Default for IntimacyFile {
    fn default() -> Self {
        let entries = PERSONALITIES
            .iter()
            .map(|p| (p.id.to_string(), IntimacyEntry::new(p.id)))
            .collect();
        Self {
            version: 1,
            migrated: false,
            entries,
        }
    }
}

// ---- 事件结果（engine 包装为 §6.1 JSON） ----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateUnlockInfo {
    pub gate: String,
    pub label: String,
    pub required_level: u8,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskUpdate {
    pub id: String,
    pub done: bool,
}

#[derive(Debug)]
pub struct EventOutcome {
    pub delta: i64,
    pub points: i64,
    pub level: u8,
    pub leveled_up: bool,
    pub unlocked: Vec<GateUnlockInfo>,
    pub achievements_granted: Vec<AchievementRecord>,
    pub tasks_updated: Vec<TaskUpdate>,
    pub capped: bool,
}

// ---- Store ----

pub struct IntimacyStore {
    path: std::path::PathBuf,
    pub file: IntimacyFile,
}

impl Default for IntimacyStore {
    fn default() -> Self {
        Self::load().unwrap_or_else(|_| Self::fallback())
    }
}

impl IntimacyStore {
    pub fn load() -> Result<Self, IntimacyStoreError> {
        let path = data_dir()?.join("intimacy.json");
        let file = if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            IntimacyFile::default()
        };
        let mut s = Self { path, file };
        s.migrate()?;
        s.persist()?;
        Ok(s)
    }

    fn fallback() -> Self {
        Self {
            path: std::env::temp_dir().join("qibai-intimacy-fallback.json"),
            file: IntimacyFile::default(),
        }
    }

    pub fn persist(&self) -> Result<(), IntimacyStoreError> {
        let raw = serde_json::to_string_pretty(&self.file)?;
        // 问题7-2：原子写（tmp+rename）。
        crate::fsutil::atomic_write_json(&self.path, &raw)?;
        Ok(())
    }

    /// 幂等迁移（§7）：`migrated=false` 时按 personas.json 的 intimacy（1–10）
    /// 把积分 seed 为该等级阈值起点；无 personas.json 时按 intimacy=1（points=0）。
    /// 已迁移则不重复播种。
    fn migrate(&mut self) -> Result<(), IntimacyStoreError> {
        if self.file.migrated {
            return Ok(());
        }
        let personas_path = data_dir()?.join("personas.json");
        let mut intimacy_of: BTreeMap<String, u8> = BTreeMap::new();
        if let Ok(raw) = std::fs::read_to_string(&personas_path) {
            if let Ok(pf) = serde_json::from_str::<crate::personas::PersonaFile>(&raw) {
                for p in pf.personas {
                    intimacy_of.insert(p.persona_id, p.intimacy.clamp(1, 10));
                }
            }
        }
        for entry in self.file.entries.values_mut() {
            let lvl = intimacy_of
                .get(&entry.persona_id)
                .copied()
                .unwrap_or(1)
                .clamp(1, 10);
            entry.points = LEVELS.get((lvl - 1) as usize).map(|i| i.min).unwrap_or(0);
            entry.level = lvl;
        }
        self.file.migrated = true;
        Ok(())
    }

    /// 跨天自动重置每日计数器（date 为 "" 的新实体也会在此落今天日期）。
    fn reset_daily_if_needed(entry: &mut IntimacyEntry) {
        let today = crate::social::today_str();
        if entry.daily.date != today {
            entry.daily = DailyState {
                date: today,
                ..DailyState::default()
            };
        }
    }

    /// 统一重算 9 个成就；新达成者写入并返回（§5）。
    fn evaluate_achievements(entry: &mut IntimacyEntry) -> Vec<AchievementRecord> {
        let c = &entry.cumulative;
        let lvl = entry.level;
        let conds: [(&str, bool); 9] = [
            ("first_greeting", c.greetings >= 1),
            ("chat_100", c.chats >= 100),
            ("click_50", c.clicks >= 50),
            ("pk_win_5", c.pk_wins >= 5),
            ("companion_7", c.companion_days >= 7),
            ("level_3", lvl >= 3),
            ("level_6", lvl >= 6),
            ("level_9", lvl >= 9),
            ("level_10", lvl >= 10),
        ];
        let mut granted = Vec::new();
        for (id, hit) in conds {
            if hit && !entry.achievements.iter().any(|a| a.id == id) {
                let rec = AchievementRecord {
                    id: id.to_string(),
                    ts: crate::storage::now_ts(),
                };
                entry.achievements.push(rec.clone());
                granted.push(rec);
            }
        }
        granted
    }

    /// 上报一次成长事件（§6.1 的 kind ∈ chat|click|task|companion|gift|pk_win）。
    /// 只改内存，调用方（engine）负责落盘。
    pub fn event(
        &mut self,
        persona_id: &str,
        kind: &str,
        detail: serde_json::Value,
    ) -> Result<EventOutcome, IntimacyStoreError> {
        let entry = self
            .file
            .entries
            .get_mut(persona_id)
            .ok_or_else(|| IntimacyStoreError::UnknownPersona(persona_id.to_string()))?;
        Self::reset_daily_if_needed(entry);
        let old_level = entry.level;
        let mut nominal: i64 = 0;
        let mut tasks_updated: Vec<TaskUpdate> = Vec::new();

        match kind {
            // +2，计入每日总上限。
            "chat" => {
                entry.cumulative.chats += 1;
                entry.daily.chats += 1;
                nominal += 2;
            }
            // +1；当日第 11 次起不再加分；当日首次 click 自动完成 click 任务（+5）。
            "click" => {
                entry.cumulative.clicks += 1;
                if entry.daily.clicks < CLICK_DAILY_LIMIT {
                    entry.daily.clicks += 1;
                    nominal += 1;
                }
                if !entry.daily.tasks.click {
                    entry.daily.tasks.click = true;
                    nominal += 5;
                    tasks_updated.push(TaskUpdate {
                        id: "click".to_string(),
                        done: true,
                    });
                }
            }
            // 完成每日任务（detail.task_id ∈ greeting|click|pk），每任务每日首次 +5。
            "task" => {
                let task_id = detail
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        IntimacyStoreError::InvalidArgument("task 事件需要 detail.task_id".into())
                    })?;
                let already = match task_id {
                    "greeting" => entry.daily.tasks.greeting,
                    "click" => entry.daily.tasks.click,
                    "pk" => entry.daily.tasks.pk,
                    other => {
                        return Err(IntimacyStoreError::InvalidArgument(format!(
                            "未知 task_id: {other}"
                        )))
                    }
                };
                if !already {
                    match task_id {
                        "greeting" => entry.daily.tasks.greeting = true,
                        "click" => entry.daily.tasks.click = true,
                        "pk" => entry.daily.tasks.pk = true,
                        _ => unreachable!(),
                    }
                    nominal += 5;
                    if task_id == "greeting" {
                        entry.cumulative.greetings += 1;
                    }
                    tasks_updated.push(TaskUpdate {
                        id: task_id.to_string(),
                        done: true,
                    });
                }
            }
            // 连续陪伴：minutes += detail.minutes（默认 10），跨过的每个阈值 +1。
            "companion" => {
                let minutes = detail
                    .get("minutes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10) as u32;
                let old = entry.daily.companion_minutes;
                let new = old + minutes;
                entry.daily.companion_minutes = new;
                let mut gained: i64 = 0;
                for &t in &COMPANION_THRESHOLDS {
                    if old < t && t <= new {
                        gained += 1;
                    }
                }
                let room = (COMPANION_DAILY_CAP - entry.daily.companion_points).max(0) as i64;
                gained = gained.min(room);
                entry.daily.companion_points += gained as u32;
                nominal += gained;
                if gained > 0 && !entry.daily.companion_day_marked {
                    entry.daily.companion_day_marked = true;
                    entry.cumulative.companion_days += 1;
                }
            }
            // +10，每角色每日仅首次（last_gift_date）。
            "gift" => {
                let today = entry.daily.date.clone();
                if entry.cumulative.last_gift_date.as_deref() != Some(today.as_str()) {
                    entry.cumulative.last_gift_date = Some(today);
                    entry.cumulative.gifts += 1;
                    nominal += 10;
                }
            }
            // +0 仅记成就；当日首次 pk 胜利自动完成 pk 任务（+5）。
            "pk_win" => {
                entry.cumulative.pk_wins += 1;
                if !entry.daily.tasks.pk {
                    entry.daily.tasks.pk = true;
                    nominal += 5;
                    tasks_updated.push(TaskUpdate {
                        id: "pk".to_string(),
                        done: true,
                    });
                }
            }
            other => {
                return Err(IntimacyStoreError::InvalidArgument(format!(
                    "未知 kind: {other}"
                )))
            }
        }

        // 每日总上限 60（§3.3）：delta = min(nominal, 60 - gained)。
        let remaining = (DAILY_POINTS_CAP - entry.daily.gained).max(0);
        let delta = nominal.min(remaining);
        let capped = delta < nominal;
        entry.daily.gained += delta;
        entry.points += delta;

        let new_level = level_of(entry.points);
        let leveled_up = new_level > old_level;
        entry.level = new_level;

        // 本次新解锁的门控（required ∈ (old_level, new_level]，含 placeholder）。
        let unlocked: Vec<GateUnlockInfo> = if leveled_up {
            GATES
                .iter()
                .filter(|g| g.required_level > old_level && g.required_level <= new_level)
                .map(|g| GateUnlockInfo {
                    gate: g.gate.to_string(),
                    label: g.label.to_string(),
                    required_level: g.required_level,
                    kind: g.kind.to_string(),
                })
                .collect()
        } else {
            Vec::new()
        };

        let achievements_granted = Self::evaluate_achievements(entry);

        Ok(EventOutcome {
            delta,
            points: entry.points,
            level: new_level,
            leveled_up,
            unlocked,
            achievements_granted,
            tasks_updated,
            capped,
        })
    }

    /// 纪念日：month 1-12 / day 1-31 校验。
    pub fn set_anniversary(
        &mut self,
        persona_id: &str,
        month: u8,
        day: u8,
    ) -> Result<(), IntimacyStoreError> {
        if !(1..=12).contains(&month) {
            return Err(IntimacyStoreError::InvalidArgument(format!(
                "month 必须在 1-12 之间（收到 {month}）"
            )));
        }
        if !(1..=31).contains(&day) {
            return Err(IntimacyStoreError::InvalidArgument(format!(
                "day 必须在 1-31 之间（收到 {day}）"
            )));
        }
        let e = self
            .file
            .entries
            .get_mut(persona_id)
            .ok_or_else(|| IntimacyStoreError::UnknownPersona(persona_id.to_string()))?;
        e.anniversary = Some(Anniversary { month, day });
        Ok(())
    }

    pub fn clear_anniversary(&mut self, persona_id: &str) -> Result<(), IntimacyStoreError> {
        let e = self
            .file
            .entries
            .get_mut(persona_id)
            .ok_or_else(|| IntimacyStoreError::UnknownPersona(persona_id.to_string()))?;
        e.anniversary = None;
        Ok(())
    }

    /// `get_intimacy_levels`（§6.1-1）：10 项，max=null 表示 ∞。
    pub fn levels_json() -> serde_json::Value {
        let levels: Vec<serde_json::Value> = LEVELS
            .iter()
            .map(|l| serde_json::json!({
                "level": l.level, "name": l.name, "desc": l.desc,
                "min": l.min, "max": l.max,
            }))
            .collect();
        serde_json::json!({ "levels": levels })
    }

    /// `get_intimacy_state`（§6.1-2）；未知 persona 报错。
    pub fn state_json(&self, persona_id: &str) -> Result<serde_json::Value, IntimacyStoreError> {
        let e = self
            .file
            .entries
            .get(persona_id)
            .ok_or_else(|| IntimacyStoreError::UnknownPersona(persona_id.to_string()))?;
        let lvl = e.level;
        let info = &LEVELS[((lvl - 1) as usize).min(9)];
        let ratio = match info.max {
            Some(max) if max > info.min => {
                ((e.points - info.min) as f64 / (max - info.min) as f64).clamp(0.0, 1.0)
            }
            _ => 1.0,
        };
        let unlocked: Vec<&str> = GATES
            .iter()
            .filter(|g| lvl >= g.required_level)
            .map(|g| g.gate)
            .collect();
        let locked: Vec<serde_json::Value> = GATES
            .iter()
            .filter(|g| lvl < g.required_level)
            .map(|g| {
                serde_json::json!({
                    "gate": g.gate, "label": g.label,
                    "required_level": g.required_level, "kind": g.kind,
                })
            })
            .collect();
        let tasks: Vec<serde_json::Value> = TASKS
            .iter()
            .map(|(id, name)| {
                let done = match *id {
                    "greeting" => e.daily.tasks.greeting,
                    "click" => e.daily.tasks.click,
                    "pk" => e.daily.tasks.pk,
                    _ => false,
                };
                serde_json::json!({ "id": id, "name": name, "done": done })
            })
            .collect();
        let achievements: Vec<serde_json::Value> = e
            .achievements
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "name": achievement_name(&a.id).unwrap_or(""),
                    "ts": a.ts,
                })
            })
            .collect();
        Ok(serde_json::json!({
            "persona_id": persona_id,
            "points": e.points,
            "level": e.level,
            "level_name": level_name(e.level),
            "progress": { "min": info.min, "max": info.max, "ratio": ratio },
            "unlocked": unlocked,
            "locked": locked,
            "daily": { "date": e.daily.date, "gained": e.daily.gained, "cap": DAILY_POINTS_CAP },
            "cumulative": {
                "chats": e.cumulative.chats,
                "clicks": e.cumulative.clicks,
                "pk_wins": e.cumulative.pk_wins,
                "companion_days": e.cumulative.companion_days,
                "greetings": e.cumulative.greetings,
                "gifts": e.cumulative.gifts,
            },
            "tasks": tasks,
            "achievements": achievements,
            "anniversary": e.anniversary,
            "sync": e.sync,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> std::path::PathBuf {
        // 调用方需在测试体内持 env_lock。
        let tmp = crate::test_support::unique_temp_dir("intimacy");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        tmp
    }
    fn teardown() {
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    /// 契约 §9-1：level_of 全部边界。
    #[test]
    fn level_of_boundaries() {
        let cases = [
            (0, 1), (119, 1), (120, 2), (279, 2), (280, 3), (479, 3), (480, 4),
            (719, 4), (720, 5), (999, 5), (1000, 6), (1329, 6), (1330, 7),
            (1719, 7), (1720, 8), (2179, 8), (2180, 9), (2719, 9), (2720, 10),
        ];
        for (points, want) in cases {
            assert_eq!(level_of(points), want, "points={points}");
        }
        assert_eq!(level_of(99999), 10);
    }

    /// 契约 §9-2：chat +2；daily.gained 累计。
    #[test]
    fn chat_event_adds_two_and_tracks_daily() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        let mut s = IntimacyStore::default();
        let o = s.event("gudian", "chat", serde_json::json!({})).unwrap();
        assert_eq!(o.delta, 2);
        assert_eq!(o.points, 2);
        assert_eq!(s.file.entries["gudian"].daily.gained, 2);
        assert_eq!(s.file.entries["gudian"].cumulative.chats, 1);
        teardown();
    }

    /// 契约 §9-3：click +1；当日第 11 次 click 不再加分。
    #[test]
    fn click_event_caps_at_ten_per_day() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        let mut s = IntimacyStore::default();
        // 第 1 次：click +1 + 自动完成 click 任务 +5
        let o = s.event("gudian", "click", serde_json::json!({})).unwrap();
        assert_eq!(o.delta, 6);
        assert!(o.tasks_updated.iter().any(|t| t.id == "click"));
        // 第 2..10 次：每次 +1
        for _ in 0..9 {
            let o = s.event("gudian", "click", serde_json::json!({})).unwrap();
            assert_eq!(o.delta, 1);
        }
        assert_eq!(s.file.entries["gudian"].daily.clicks, 10);
        // 第 11 次：click 与任务均无新分
        let o = s.event("gudian", "click", serde_json::json!({})).unwrap();
        assert_eq!(o.delta, 0);
        teardown();
    }

    /// 契约 §9-4：每日总上限 60；已得 59 时 chat 只加 1（capped），已得 60 时 delta=0。
    #[test]
    fn daily_total_cap_sixty() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        let mut s = IntimacyStore::default();
        // 先落今天日期（新实体 date="" 会在首次事件时被重置），再模拟已得 59 分
        let e = s.file.entries.get_mut("gudian").unwrap();
        e.daily.date = crate::social::today_str();
        e.daily.gained = 59;
        let o = s.event("gudian", "chat", serde_json::json!({})).unwrap();
        assert_eq!(o.delta, 1);
        assert!(o.capped);
        assert_eq!(s.file.entries["gudian"].daily.gained, 60);
        let o = s.event("gudian", "chat", serde_json::json!({})).unwrap();
        assert_eq!(o.delta, 0);
        assert!(o.capped);
        teardown();
    }

    /// 契约 §9-5：companion 递减公式与当日 10 分封顶。
    #[test]
    fn companion_decay_schedule_and_daily_cap() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        let mut s = IntimacyStore::default();
        let o = s.event("gudian", "companion", serde_json::json!({})).unwrap(); // 10min
        assert_eq!(o.delta, 1);
        s.event("gudian", "companion", serde_json::json!({})).unwrap(); // 20min
        let o = s.event("gudian", "companion", serde_json::json!({})).unwrap(); // 30min
        assert_eq!(o.delta, 1);
        assert_eq!(s.file.entries["gudian"].daily.companion_points, 3);
        let o = s.event("gudian", "companion", serde_json::json!({})).unwrap(); // 40min
        assert_eq!(o.delta, 0);
        let o = s.event("gudian", "companion", serde_json::json!({})).unwrap(); // 50min
        assert_eq!(o.delta, 1);
        // 50 → 180：再 13 次 +10min，阈值 70/90/120/150/180 各 +1，共 9
        for _ in 0..13 {
            s.event("gudian", "companion", serde_json::json!({})).unwrap();
        }
        assert_eq!(s.file.entries["gudian"].daily.companion_minutes, 180);
        assert_eq!(s.file.entries["gudian"].daily.companion_points, 9);
        // 190min：无新阈值
        let o = s.event("gudian", "companion", serde_json::json!({})).unwrap();
        assert_eq!(o.delta, 0);
        // companion_days 当日只记一次
        assert_eq!(s.file.entries["gudian"].cumulative.companion_days, 1);
        teardown();
    }

    /// 契约 §9-6：task 首次 +5、同日重复 delta=0、跨天重置。
    #[test]
    fn task_completion_dedup_and_cross_day_reset() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        let mut s = IntimacyStore::default();
        let o = s.event("gudian", "task", serde_json::json!({"task_id":"greeting"})).unwrap();
        assert_eq!(o.delta, 5);
        assert!(o.tasks_updated.iter().any(|t| t.id == "greeting"));
        assert_eq!(s.file.entries["gudian"].cumulative.greetings, 1);
        // 同日重复
        let o = s.event("gudian", "task", serde_json::json!({"task_id":"greeting"})).unwrap();
        assert_eq!(o.delta, 0);
        // pk 任务（经 task 事件）
        let o = s.event("gudian", "task", serde_json::json!({"task_id":"pk"})).unwrap();
        assert_eq!(o.delta, 5);
        // 跨天：daily 全部归零，greeting 可再次完成
        s.file.entries
            .get_mut("gudian")
            .unwrap()
            .daily
            .date = "1999-01-01".to_string();
        let o = s.event("gudian", "task", serde_json::json!({"task_id":"greeting"})).unwrap();
        assert_eq!(o.delta, 5);
        assert_eq!(s.file.entries["gudian"].daily.gained, 5);
        assert!(!s.file.entries["gudian"].daily.tasks.pk); // pk 任务也已重置
        teardown();
    }

    /// 契约 §9-7：gift +10；同日第二次 delta=0（last_gift_date）。
    #[test]
    fn gift_first_per_day() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        let mut s = IntimacyStore::default();
        let o = s.event("gudian", "gift", serde_json::json!({})).unwrap();
        assert_eq!(o.delta, 10);
        let e = &s.file.entries["gudian"];
        assert_eq!(e.cumulative.gifts, 1);
        assert!(e.cumulative.last_gift_date.is_some());
        let o = s.event("gudian", "gift", serde_json::json!({})).unwrap();
        assert_eq!(o.delta, 0);
        teardown();
    }

    /// 契约 §9-8（引擎侧写事件日志见 engine 测试）：120 分升 L2、unlocked 含新门控。
    #[test]
    fn level_up_reaches_l2_and_unlocks_gates() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        let mut s = IntimacyStore::default();
        s.file.entries.get_mut("gudian").unwrap().points = 118;
        let o = s.event("gudian", "chat", serde_json::json!({})).unwrap();
        assert_eq!(o.delta, 2);
        assert_eq!(o.points, 120);
        assert!(o.leveled_up);
        assert_eq!(o.level, 2);
        let gates: Vec<&str> = o.unlocked.iter().map(|u| u.gate.as_str()).collect();
        assert!(gates.contains(&"custom_nickname"));
        assert!(gates.contains(&"daily_greeting"));
        assert!(gates.contains(&"water_reminder"));
        teardown();
    }

    /// 契约 §9-9：21 键全表 + assistant_skills 验收断言。
    #[test]
    fn gate_table_full_and_assistant_skills() {
        assert_eq!(GATES.len(), 21);
        for g in &GATES {
            assert_eq!(gate_required(g.gate), Some(g.required_level));
        }
        assert_eq!(gate_required("assistant_skills"), Some(4));
        // L1 false / L4 true（验收示例）
        assert!(!is_gate_open(1, "assistant_skills"));
        assert!(is_gate_open(4, "assistant_skills"));
        assert!(is_gate_open(5, "assistant_skills"));
        assert!(is_gate_open(1, "basic_chat"));
        assert!(!is_gate_open(10, "no_such_gate"));
        teardown();
    }

    /// 契约 §9-10：成就判定（chat_100 / level_10 / companion_7）。
    #[test]
    fn achievements_grant_on_conditions() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        let mut s = IntimacyStore::default();
        // chat_100
        s.file.entries.get_mut("gudian").unwrap().cumulative.chats = 99;
        let o = s.event("gudian", "chat", serde_json::json!({})).unwrap();
        assert!(o.achievements_granted.iter().any(|a| a.id == "chat_100"));
        // level_10
        s.file.entries.get_mut("gudian").unwrap().points = 2720;
        let o = s.event("gudian", "chat", serde_json::json!({})).unwrap();
        assert_eq!(o.level, 10);
        assert!(o.achievements_granted.iter().any(|a| a.id == "level_10"));
        // companion_7：模拟 7 个"有 companion 得分的新的一天"
        for _ in 0..7 {
            s.file.entries
                .get_mut("gudian")
                .unwrap()
                .daily
                .date = "1999-01-01".to_string();
            s.event("gudian", "companion", serde_json::json!({})).unwrap();
        }
        assert_eq!(s.file.entries["gudian"].cumulative.companion_days, 7);
        // 第 7 次 companion 事件末尾即完成判定并落成就
        assert!(s
            .file
            .entries["gudian"]
            .achievements
            .iter()
            .any(|a| a.id == "companion_7"));
        // 同日再事件不重复发成就
        let o = s.event("gudian", "chat", serde_json::json!({})).unwrap();
        assert!(o.achievements_granted.is_empty());
        teardown();
    }

    /// 契约 §9-11：personas.json intimacy=7 → points=1330/level=7；连跑 3 次不变。
    #[test]
    fn migration_from_personas_is_idempotent() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        {
            let mut ps = crate::personas::PersonaStore::load(1).unwrap();
            ps.file.personas[0].intimacy = 7; // gudian
            ps.persist().unwrap();
        }
        let s1 = IntimacyStore::load().unwrap();
        assert!(s1.file.migrated);
        assert_eq!(s1.file.entries["gudian"].points, 1330);
        assert_eq!(s1.file.entries["gudian"].level, 7);
        assert_eq!(s1.file.entries["dushi"].points, 0);
        drop(s1);
        // 连跑 3 次结果一致
        for _ in 0..3 {
            let s = IntimacyStore::load().unwrap();
            assert_eq!(s.file.entries["gudian"].points, 1330);
            assert_eq!(s.file.entries["gudian"].level, 7);
            assert_eq!(s.file.entries.len(), 5);
        }
        teardown();
    }

    /// 契约 §9-11 附：缺字段旧文件 serde default 补齐，不报错不漂移。
    #[test]
    fn old_file_without_new_fields_gets_defaults() {
        let _g = crate::test_support::env_lock();
        let tmp = setup();
        std::fs::write(
            tmp.join("intimacy.json"),
            r#"{"version":1,"migrated":true,"entries":{"gudian":{"persona_id":"gudian","points":10,"level":1}}}"#,
        )
        .unwrap();
        let s = IntimacyStore::load().unwrap();
        let e = &s.file.entries["gudian"];
        assert_eq!(e.points, 10);
        assert_eq!(e.daily.gained, 0);
        assert_eq!(e.daily.tasks.greeting, false);
        assert!(e.achievements.is_empty());
        assert!(e.anniversary.is_none());
        assert!(e.sync.server_id.is_none());
        let s2 = IntimacyStore::load().unwrap();
        assert_eq!(s2.file.entries["gudian"].points, 10);
        teardown();
    }

    /// 契约 §9-12：daily 计数跨天重置。
    #[test]
    fn daily_counters_reset_on_new_day() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        let mut s = IntimacyStore::default();
        s.event("gudian", "chat", serde_json::json!({})).unwrap();
        s.event("gudian", "click", serde_json::json!({})).unwrap();
        assert!(s.file.entries["gudian"].daily.gained > 0);
        // daily.date 置旧日期后再事件
        s.file.entries
            .get_mut("gudian")
            .unwrap()
            .daily
            .date = "1999-01-01".to_string();
        let o = s.event("gudian", "chat", serde_json::json!({})).unwrap();
        assert_eq!(o.delta, 2);
        let e = &s.file.entries["gudian"];
        assert_eq!(e.daily.gained, 2);
        assert_eq!(e.daily.clicks, 0);
        assert!(!e.daily.tasks.click);
        teardown();
    }

    /// 纪念日校验：month 1-12 / day 1-31。
    #[test]
    fn anniversary_validation_and_roundtrip() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        let mut s = IntimacyStore::default();
        assert!(s.set_anniversary("gudian", 0, 15).is_err());
        assert!(s.set_anniversary("gudian", 13, 15).is_err());
        assert!(s.set_anniversary("gudian", 6, 32).is_err());
        assert!(s.set_anniversary("gudian", 6, 31).is_ok());
        assert_eq!(
            s.file.entries["gudian"].anniversary,
            Some(Anniversary { month: 6, day: 31 })
        );
        s.clear_anniversary("gudian").unwrap();
        assert!(s.file.entries["gudian"].anniversary.is_none());
        teardown();
    }

    /// levels/state JSON 形状（§6.1-1/2）。
    #[test]
    fn levels_and_state_json_shape() {
        let _g = crate::test_support::env_lock();
        let _tmp = setup();
        let s = IntimacyStore::default();
        let lv = IntimacyStore::levels_json();
        assert_eq!(lv["levels"].as_array().unwrap().len(), 10);
        assert_eq!(lv["levels"][0]["name"], "初识");
        assert_eq!(lv["levels"][9]["max"], serde_json::Value::Null);
        let st = s.state_json("gudian").unwrap();
        assert_eq!(st["level"], 1);
        assert_eq!(st["level_name"], "初识");
        assert_eq!(st["progress"]["min"], 0);
        assert!(st["unlocked"].as_array().unwrap().contains(&serde_json::json!("basic_chat")));
        assert!(!st["locked"].as_array().unwrap().is_empty());
        assert_eq!(st["daily"]["cap"], 60);
        assert_eq!(st["tasks"].as_array().unwrap().len(), 3);
        assert!(s.state_json("ghost").is_err());
        teardown();
    }
}
