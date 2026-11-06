//! P0 角色实体化：5 个形象从"皮肤"升级为独立角色实体。
//!
//! 文件：`<data_dir>/personas.json`（整文件读写，与 settings.json/conversation.json 同模式）。
//! 隐私本地优先：不上传；`sync` 字段本期只写不读，预留远期联网。
//!
//! 迁移（幂等）：
//! - personas.json 不存在时首次初始化：5 个默认实体，active_persona = "gudian"，
//!   gudian.intimacy 取旧 settings.intimacy_level；
//! - 旧 conversation.json 里无角色归属的记录，在加载时归入 active persona 的记忆；
//!   用 `migrated_conversation` 标志位保证重复执行结果一致。

use crate::persona::{is_known_persona, personality_of, PERSONALITIES};
use crate::settings::data_dir;
use crate::voice_profile::{default_profile_for, VoiceProfile};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 每实体记忆上限（环形裁剪：超出即丢最旧）。
pub const MEMORY_CAP: usize = 50;

#[derive(Debug, Error)]
pub enum PersonaStoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("settings error: {0}")]
    Settings(#[from] crate::settings::SettingsError),
    #[error("unknown persona: {0}")]
    UnknownPersona(String),
}

/// 一条记忆片段（与 conversation.json 记录同构，但归属某个角色）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryEntry {
    pub role: String,
    pub content: String,
    pub ts: u64,
}

/// 性格参数的结构化摘要（persona.rs 性格表的投影）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Traits {
    pub tone: String,
    pub catchphrase: String,
    pub interaction: String,
}

/// 远期联网预留：本期只写不读。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncState {
    pub server_id: Option<String>,
    pub dirty: bool,
    pub version: u64,
}

impl Default for SyncState {
    fn default() -> Self {
        Self {
            server_id: None,
            dirty: false,
            version: 0,
        }
    }
}

/// 角色实体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersonaEntity {
    pub persona_id: String,
    pub display_name: String,
    /// 用户可重命名的昵称。
    pub nickname: String,
    pub traits: Traits,
    /// 迭代⑤：该角色专属音色档案（旧文件缺字段时 serde default 补 Auto，迁移幂等）。
    #[serde(default)]
    pub voice_profile: VoiceProfile,
    /// 与用户的亲密度 1-10（首次迁移自旧 settings.intimacy_level）。
    pub intimacy: u8,
    pub memory: Vec<MemoryEntry>,
    pub sync: SyncState,
}

/// personas.json 顶层结构。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersonaFile {
    pub version: u32,
    pub active_persona: String,
    pub personas: Vec<PersonaEntity>,
    /// 内部迁移标志：旧 conversation.json 是否已归入 active persona 记忆。
    #[serde(default)]
    pub migrated_conversation: bool,
}

/// 按性格表造 5 个默认实体；`gudian` 的 intimacy 取旧 settings 亲密度。
fn default_file(intimacy: u8) -> PersonaFile {
    let personas = PERSONALITIES
        .iter()
        .map(|p| PersonaEntity {
            persona_id: p.id.to_string(),
            display_name: p.display_name.to_string(),
            nickname: p.display_name.to_string(),
            traits: Traits {
                tone: p.tone.to_string(),
                catchphrase: p.catchphrase.to_string(),
                interaction: p.interaction.to_string(),
            },
            intimacy: if p.id == "gudian" { intimacy } else { 1 },
            // 存储一律固化 Auto（运行时按性格映射默认值），不固化性格表数值 →
            // 旧 personas.json 缺字段时 serde default 自动补 Auto，迁移幂等。
            voice_profile: VoiceProfile::default(),
            memory: Vec::new(),
            sync: SyncState::default(),
        })
        .collect();
    PersonaFile {
        version: 1,
        active_persona: "gudian".to_string(),
        personas,
        migrated_conversation: false,
    }
}

pub struct PersonaStore {
    path: std::path::PathBuf,
    pub file: PersonaFile,
}

impl Default for PersonaStore {
    fn default() -> Self {
        Self::load(1).unwrap_or_else(|_| Self::fallback(1))
    }
}

impl PersonaStore {
    /// 读取 personas.json；不存在（或损坏）时用默认 5 实体初始化并落盘。
    pub fn load(default_intimacy: u8) -> Result<Self, PersonaStoreError> {
        let path = data_dir()?.join("personas.json");
        let file = if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            serde_json::from_str(&raw).unwrap_or_else(|_| default_file(default_intimacy))
        } else {
            default_file(default_intimacy)
        };
        let store = Self { path, file };
        store.persist()?;
        Ok(store)
    }

    /// 数据目录不可用时的兜底（与 ConversationStore::empty 同思路，绝不 panic）。
    fn fallback(intimacy: u8) -> Self {
        Self {
            path: std::env::temp_dir().join("qibai-personas-fallback.json"),
            file: default_file(intimacy),
        }
    }

    pub fn active_persona(&self) -> &str {
        &self.file.active_persona
    }

    pub fn get(&self, id: &str) -> Option<&PersonaEntity> {
        self.file.personas.iter().find(|p| p.persona_id == id)
    }

    pub fn active(&self) -> &PersonaEntity {
        self.get(&self.file.active_persona)
            .unwrap_or(&self.file.personas[0])
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut PersonaEntity, PersonaStoreError> {
        self.file
            .personas
            .iter_mut()
            .find(|p| p.persona_id == id)
            .ok_or_else(|| PersonaStoreError::UnknownPersona(id.to_string()))
    }

    /// 切换当前角色；未知 id 报错。
    pub fn set_active(&mut self, id: &str) -> Result<(), PersonaStoreError> {
        if !is_known_persona(id) {
            return Err(PersonaStoreError::UnknownPersona(id.to_string()));
        }
        self.file.active_persona = id.to_string();
        self.persist()
    }

    /// 重命名昵称；未知 id 报错。
    pub fn rename(&mut self, id: &str, nickname: &str) -> Result<(), PersonaStoreError> {
        self.get_mut(id)?.nickname = nickname.to_string();
        self.persist()
    }

    /// 设置某角色的音色档案；落盘前 sanitize；未知 id 报 UnknownPersona。
    pub fn set_voice_profile(
        &mut self,
        id: &str,
        profile: VoiceProfile,
    ) -> Result<(), PersonaStoreError> {
        let e = self.get_mut(id)?;
        e.voice_profile = profile.sanitize();
        self.persist()
    }

    /// 解析生效音色：实体存在且 mode==Custom → 存值 sanitize；
    /// 否则（Auto 或实体不存在）→ default_profile_for(id)，未知 id 回退 gudian。
    pub fn resolved_voice_profile(&self, id: &str) -> VoiceProfile {
        match self.get(id) {
            Some(e) if e.voice_profile.mode == crate::voice_profile::VoiceProfileMode::Custom => {
                e.voice_profile.sanitize()
            }
            _ => default_profile_for(id),
        }
    }

    /// 向某角色追加一条记忆；超过 [`MEMORY_CAP`] 时环形裁剪（丢最旧）。
    pub fn append_memory(
        &mut self,
        id: &str,
        role: &str,
        content: &str,
    ) -> Result<(), PersonaStoreError> {
        let e = self.get_mut(id)?;
        e.memory.push(MemoryEntry {
            role: role.to_string(),
            content: content.to_string(),
            ts: crate::storage::now_ts(),
        });
        if e.memory.len() > MEMORY_CAP {
            let drop = e.memory.len() - MEMORY_CAP;
            e.memory.drain(0..drop);
        }
        Ok(())
    }

    /// 把旧 conversation.json 的全部记录归入 active persona 记忆（仅首次迁移调用）。
    pub fn migrate_conversation<I, S1, S2>(&mut self, records: I)
    where
        I: IntoIterator<Item = (S1, S2)>,
        S1: AsRef<str>,
        S2: AsRef<str>,
    {
        let active = self.file.active_persona.clone();
        for (role, content) in records {
            let _ = self.append_memory(&active, role.as_ref(), content.as_ref());
        }
        self.file.migrated_conversation = true;
        let _ = self.persist();
    }

    /// v18.8-fix28：清空所有角色记忆条目并落盘（测试/隐私重置）。返回清除条数。
    pub fn clear_all_memory(&mut self) -> Result<usize, PersonaStoreError> {
        let mut n = 0;
        for p in self.file.personas.iter_mut() {
            n += p.memory.len();
            p.memory.clear();
        }
        self.persist()?;
        Ok(n)
    }

    pub fn persist(&self) -> Result<(), PersonaStoreError> {
        let raw = serde_json::to_string_pretty(&self.file)?;
        // 问题7-2：原子写（tmp+rename）。
        crate::fsutil::atomic_write_json(&self.path, &raw)?;
        Ok(())
    }

    /// 性格表投影（供 engine/social 取 display_name 等）。
    pub fn personality(id: &str) -> &'static crate::persona::PersonaPersonality {
        personality_of(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> std::path::PathBuf {
        // 调用方需在测试体内持 env_lock。
        let tmp = crate::test_support::unique_temp_dir("personas");
        unsafe { std::env::set_var("QIBAI_DATA_DIR", &tmp) };
        tmp
    }

    fn teardown() {
        unsafe { std::env::remove_var("QIBAI_DATA_DIR") };
    }

    #[test]
    fn fresh_init_creates_five_default_entities() {
        let _g = crate::test_support::env_lock();
        let tmp = setup();
        let s = PersonaStore::load(7).unwrap();
        assert_eq!(s.file.personas.len(), 5);
        assert_eq!(s.file.active_persona, "gudian");
        // gudian 亲密度迁移自旧 settings.intimacy_level=7，其余默认 1
        assert_eq!(s.get("gudian").unwrap().intimacy, 7);
        assert_eq!(s.get("dushi").unwrap().intimacy, 1);
        assert!(tmp.join("personas.json").exists());
        teardown();
    }

    #[test]
    fn migration_is_idempotent() {
        let _g = crate::test_support::env_lock();
        let tmp = setup();
        // 第一次加载：迁移两条旧对话
        {
            let mut s = PersonaStore::load(2).unwrap();
            assert!(!s.file.migrated_conversation);
            s.migrate_conversation([("user", "你好"), ("assistant", "你好呀")]);
        }
        // 第二次加载：标志位已置，不重复迁移
        {
            let s = PersonaStore::load(2).unwrap();
            assert!(s.file.migrated_conversation);
            assert_eq!(s.get("gudian").unwrap().memory.len(), 2);
            assert_eq!(s.get("gudian").unwrap().intimacy, 2);
        }
        // 第三次：依然是 2 条，不翻倍
        {
            let s = PersonaStore::load(2).unwrap();
            assert_eq!(s.get("gudian").unwrap().memory.len(), 2);
        }
        let _ = tmp;
        teardown();
    }

    #[test]
    fn memory_rings_at_50() {
        let _g = crate::test_support::env_lock();
        let tmp = setup();
        let mut s = PersonaStore::load(1).unwrap();
        for i in 0..60 {
            s.append_memory("gudian", "user", &format!("msg-{i}")).unwrap();
        }
        let m = &s.get("gudian").unwrap().memory;
        assert_eq!(m.len(), MEMORY_CAP);
        // 最旧的 10 条被裁掉，保留 msg-10..=msg-59
        assert_eq!(m[0].content, "msg-10");
        assert_eq!(m[49].content, "msg-59");
        let _ = tmp;
        teardown();
    }

    #[test]
    fn unknown_persona_rejected() {
        let _g = crate::test_support::env_lock();
        let tmp = setup();
        let mut s = PersonaStore::load(1).unwrap();
        assert!(s.set_active("nosuch").is_err());
        assert!(s.rename("nosuch", "x").is_err());
        let _ = tmp;
        teardown();
    }

    // ---- 迭代⑤：voice_profile 迁移幂等与解析 ----

    /// 手工写旧版 personas.json（实体无 voice_profile 字段）→ 加载后实体 mode==Auto。
    #[test]
    fn legacy_personas_json_loads_with_default_auto_voice_profile() {
        let _g = crate::test_support::env_lock();
        let tmp = setup();
        let legacy = r#"{
            "version": 1,
            "active_persona": "gudian",
            "personas": [
                {"persona_id":"gudian","display_name":"古典佳人","nickname":"古典佳人",
                 "traits":{"tone":"温婉","catchphrase":"……（轻声）","interaction":"x"},"intimacy":5,"memory":[],"sync":{"server_id":null,"dirty":false,"version":0}},
                {"persona_id":"dushi","display_name":"都市女性","nickname":"都市女性",
                 "traits":{"tone":"知性","catchphrase":"嗯","interaction":"y"},"intimacy":1,"memory":[],"sync":{"server_id":null,"dirty":false,"version":0}}
            ]
        }"#;
        std::fs::write(tmp.join("personas.json"), legacy).unwrap();
        let s = PersonaStore::load(1).unwrap();
        // 旧文件缺字段 → serde default 补 VoiceProfile::default()（Auto）
        assert_eq!(
            s.get("gudian").unwrap().voice_profile.mode,
            crate::voice_profile::VoiceProfileMode::Auto
        );
        assert_eq!(
            s.get("dushi").unwrap().voice_profile.mode,
            crate::voice_profile::VoiceProfileMode::Auto
        );
        // Auto 实体解析出性格表默认音色
        let r = s.resolved_voice_profile("dushi");
        assert_eq!(r.cloud_speaker, "zh_female_shuangkuaisisi_emo_v2_mars_bigtts");
        let _ = tmp;
        teardown();
    }

    #[test]
    fn voice_profile_migration_roundtrip_and_idempotent() {
        let _g = crate::test_support::env_lock();
        let tmp = setup();
        // 连续 load 两次结构一致（fresh init → 落盘 → 再加载）
        {
            let s = PersonaStore::load(3).unwrap();
            let raw1 = std::fs::read_to_string(tmp.join("personas.json")).unwrap();
            let s2 = PersonaStore::load(3).unwrap();
            let raw2 = std::fs::read_to_string(tmp.join("personas.json")).unwrap();
            assert_eq!(raw1, raw2);
            assert_eq!(s.file, s2.file);
        }
        // set_voice_profile：sanitize + 落盘 + 重载 round-trip
        let custom = crate::voice_profile::VoiceProfile {
            mode: crate::voice_profile::VoiceProfileMode::Custom,
            pitch: 50.0,
            rate: 5.0,
            engine_voice_id: "my-espeak-voice".into(),
            cloud_speaker: "my-speaker".into(),
        };
        {
            let mut s = PersonaStore::load(3).unwrap();
            s.set_voice_profile("huopo", custom.clone()).unwrap();
            // 落盘前 sanitize
            let e = s.get("huopo").unwrap();
            assert_eq!(e.voice_profile.pitch, 10.0);
            assert_eq!(e.voice_profile.rate, 2.0);
            // 未知 id 报 UnknownPersona
            assert!(s.set_voice_profile("nosuch", custom.clone()).is_err());
        }
        {
            let s = PersonaStore::load(3).unwrap();
            let e = s.get("huopo").unwrap();
            assert_eq!(e.voice_profile.mode, crate::voice_profile::VoiceProfileMode::Custom);
            assert_eq!(e.voice_profile.pitch, 10.0);
            assert_eq!(e.voice_profile.engine_voice_id, "my-espeak-voice");
            assert_eq!(e.voice_profile.cloud_speaker, "my-speaker");
            // Custom → 存值 sanitize；Auto 角色仍走性格表
            let r = s.resolved_voice_profile("huopo");
            assert_eq!(r.cloud_speaker, "my-speaker");
            let r2 = s.resolved_voice_profile("gudian");
            assert_eq!(r2.cloud_speaker, "zh_female_wenrouxiaoya_moon_bigtts");
            // 未知 id → gudian 回退
            assert_eq!(s.resolved_voice_profile("nosuch").cloud_speaker, "zh_female_wenrouxiaoya_moon_bigtts");
        }
        let _ = tmp;
        teardown();
    }
}
