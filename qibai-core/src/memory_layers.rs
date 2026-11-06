//! 记忆分层与修订检查（Personalization & Memory 轻量落地）。
//!
//! 借鉴 Qwen-Audio-Agent（docs/reference/memory.md / architecture deep-dive）：
//! - 四层：核心规则（core rules）> 助手画像（assistant profile）>
//!   显式偏好（explicit preferences）> 长期记忆（long-term / inferred）。
//! - 优先级：**当前请求 > 显式偏好 > 推断偏好**——
//!   用户这一轮说的话永远赢；用户明说的偏好赢过 LLM 猜的。
//! - **修订检查（revise-check）**：更新必须命中唯一预期片段才覆盖，
//!   缺失/歧义即失败——防止 LLM 一轮话把用户长期记忆静默覆盖。
//!
//! 对齐元气小小现状：intimacy.rs 已有 `preference_memory` 亲密度门禁与
//! DailyState/CumulativeState 持久化；本模块是**叠加在其上的组装与冲突规则**，
//! 不改 intimacy 存储格式（增强不重造）。

use serde::{Deserialize, Serialize};

/// 偏好来源：用户明说 vs LLM 推断。冲突时显式永远赢。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// 用户明确说过（如「以后别叫我宝贝」）。
    Explicit,
    /// LLM 从对话推断（如「似乎喜欢安静语气」）。
    Inferred,
}

/// 一条偏好/记忆条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreferenceEntry {
    pub key: String,
    pub value: String,
    pub provenance: Provenance,
    /// 版本号：每次成功修订 +1，乐观并发用。
    pub version: u32,
}

impl PreferenceEntry {
    pub fn new_explicit(key: &str, value: &str) -> Self {
        Self {
            key: key.to_string(),
            value: value.to_string(),
            provenance: Provenance::Explicit,
            version: 1,
        }
    }
    pub fn new_inferred(key: &str, value: &str) -> Self {
        Self {
            key: key.to_string(),
            value: value.to_string(),
            provenance: Provenance::Inferred,
            version: 1,
        }
    }
}

/// 修订错误。
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ReviseError {
    #[error("entry not found: {0}")]
    NotFound(String),
    #[error("expected fragment not found in stored value — refusing to overwrite: {0}")]
    FragmentMissing(String),
    #[error("expected fragment ambiguous (matches multiple times) — refusing to overwrite: {0}")]
    FragmentAmbiguous(String),
    #[error("inferred entry cannot overwrite explicit preference: {0}")]
    CannotClobberExplicit(String),
}

/// 记忆快照：组装 system 上下文的四层原料。
#[derive(Debug, Clone, Default)]
pub struct MemorySnapshot {
    /// 核心规则（不可变系统行为）。
    pub core_rules: String,
    /// 助手画像（当前 persona）。
    pub assistant_profile: String,
    /// 偏好条目（显式+推断混合存储，组装时按优先级排）。
    pub preferences: Vec<PreferenceEntry>,
}

impl MemorySnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    /// 写入一条偏好。规则：
    /// - 同 key 已存在 Explicit 时，Inferred 写入直接拒绝（防覆盖）；
    /// - 同 key 已存在 Inferred 时，Explicit 升级覆盖（显式赢）。
    pub fn upsert(&mut self, entry: PreferenceEntry) -> Result<(), ReviseError> {
        if let Some(existing) = self.preferences.iter_mut().find(|p| p.key == entry.key) {
            if existing.provenance == Provenance::Explicit
                && entry.provenance == Provenance::Inferred
            {
                return Err(ReviseError::CannotClobberExplicit(entry.key));
            }
            existing.value = entry.value;
            existing.provenance = entry.provenance;
            existing.version += 1;
            return Ok(());
        }
        self.preferences.push(entry);
        Ok(())
    }

    /// 修订检查：仅当当前 value 包含**唯一一处** `expected_fragment` 时才覆盖。
    /// 对齐 qwen：replace 定位唯一片段，缺失/歧义即安全失败。
    pub fn revise(
        &mut self,
        key: &str,
        expected_fragment: &str,
        new_value: &str,
    ) -> Result<u32, ReviseError> {
        let entry = self
            .preferences
            .iter_mut()
            .find(|p| p.key == key)
            .ok_or_else(|| ReviseError::NotFound(key.to_string()))?;
        let count = entry.value.matches(expected_fragment).count();
        if count == 0 {
            return Err(ReviseError::FragmentMissing(key.to_string()));
        }
        if count > 1 {
            return Err(ReviseError::FragmentAmbiguous(key.to_string()));
        }
        entry.value = new_value.to_string();
        entry.version += 1;
        Ok(entry.version)
    }

    /// 组装 system 上下文片段：顺序 = 核心规则 → 画像 → 显式偏好 → 推断记忆。
    /// `current_request` 永远放在最前（当前请求 > 一切静态记忆）。
    pub fn assemble_system_prompt(&self, current_request: &str) -> String {
        let mut out = String::new();
        // 当前请求永远最优先
        if !current_request.trim().is_empty() {
            out.push_str("[本轮请求]\n");
            out.push_str(current_request.trim());
            out.push_str("\n\n");
        }
        if !self.core_rules.is_empty() {
            out.push_str("[核心规则]\n");
            out.push_str(&self.core_rules);
            out.push_str("\n\n");
        }
        if !self.assistant_profile.is_empty() {
            out.push_str("[助手画像]\n");
            out.push_str(&self.assistant_profile);
            out.push_str("\n\n");
        }
        let mut explicit: Vec<&PreferenceEntry> = self
            .preferences
            .iter()
            .filter(|p| p.provenance == Provenance::Explicit)
            .collect();
        let mut inferred: Vec<&PreferenceEntry> = self
            .preferences
            .iter()
            .filter(|p| p.provenance == Provenance::Inferred)
            .collect();
        explicit.sort_by(|a, b| a.key.cmp(&b.key));
        inferred.sort_by(|a, b| a.key.cmp(&b.key));
        if !explicit.is_empty() {
            out.push_str("[用户明确偏好]\n");
            for p in &explicit {
                out.push_str(&format!("- {}: {}\n", p.key, p.value));
            }
            out.push('\n');
        }
        if !inferred.is_empty() {
            out.push_str("[长期记忆（推断，仅供参考）]\n");
            for p in &inferred {
                out.push_str(&format!("- {}: {}\n", p.key, p.value));
            }
        }
        out.trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_beats_inferred_on_conflict() {
        let mut s = MemorySnapshot::new();
        s.upsert(PreferenceEntry::new_inferred("称呼", "宝贝")).unwrap();
        // 用户后来明说别叫宝贝 → 显式升级覆盖推断
        s.upsert(PreferenceEntry::new_explicit("称呼", "直呼名字"))
            .unwrap();
        let e = s.preferences.iter().find(|p| p.key == "称呼").unwrap();
        assert_eq!(e.provenance, Provenance::Explicit);
        assert_eq!(e.value, "直呼名字");
    }

    #[test]
    fn inferred_cannot_clobber_explicit() {
        let mut s = MemorySnapshot::new();
        s.upsert(PreferenceEntry::new_explicit("语气", "安静克制")).unwrap();
        let r = s.upsert(PreferenceEntry::new_inferred("语气", "活泼"));
        assert!(matches!(r, Err(ReviseError::CannotClobberExplicit(_))));
        // 原值不动
        let e = s.preferences.iter().find(|p| p.key == "语气").unwrap();
        assert_eq!(e.value, "安静克制");
    }

    #[test]
    fn revise_check_happy_path() {
        let mut s = MemorySnapshot::new();
        s.upsert(PreferenceEntry::new_explicit("作息", "23点睡"))
            .unwrap();
        let v = s.revise("作息", "23点睡", "23点半睡").unwrap();
        assert_eq!(v, 2);
    }

    #[test]
    fn revise_check_rejects_missing_fragment() {
        let mut s = MemorySnapshot::new();
        s.upsert(PreferenceEntry::new_explicit("作息", "23点睡"))
            .unwrap();
        let r = s.revise("作息", "22点睡", "改了");
        assert!(matches!(r, Err(ReviseError::FragmentMissing(_))));
        // 未覆盖
        assert_eq!(
            s.preferences.iter().find(|p| p.key == "作息").unwrap().value,
            "23点睡"
        );
    }

    #[test]
    fn revise_check_rejects_ambiguous_fragment() {
        let mut s = MemorySnapshot::new();
        s.upsert(PreferenceEntry::new_explicit("笔记", "备注：重复 重复"))
            .unwrap();
        let r = s.revise("笔记", "重复", "新");
        assert!(matches!(r, Err(ReviseError::FragmentAmbiguous(_))));
    }

    #[test]
    fn assembly_order_current_request_first() {
        let mut s = MemorySnapshot::new();
        s.core_rules = "不撒谎".to_string();
        s.assistant_profile = "温柔桌宠".to_string();
        s.upsert(PreferenceEntry::new_inferred("喜好", "安静")).unwrap();
        s.upsert(PreferenceEntry::new_explicit("称呼", "名字")).unwrap();
        let prompt = s.assemble_system_prompt("现在提醒我喝水");
        // 当前请求在最前
        assert!(prompt.starts_with("[本轮请求]\n现在提醒我喝水"));
        // 显式偏好排在推断记忆之前
        let idx_explicit = prompt.find("[用户明确偏好]").unwrap();
        let idx_inferred = prompt.find("[长期记忆").unwrap();
        assert!(idx_explicit < idx_inferred);
    }
}
