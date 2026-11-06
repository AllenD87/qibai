//! 专家体系（T5，契约 §3.4）：人设提示词包 + 技能白名单 + 音色/形象绑定。
//!
//! - 专家 = type=expert 的插件包（manifest.system_prompt + skills 白名单 + persona_binding）。
//! - 装配：base system prompt + 专家追加段；白名单过滤 LLM 工具表。
//! - voice：已知 persona id（gudian/dushi/…）经 `voice_profile::default_profile_for` 解析；
//!   未知字符串原样透传（本期不做强制映射）。

use crate::plugin_manifest::{Manifest, PersonaBinding};
use crate::settings::PluginSettings;

/// 专家条目。
#[derive(Debug, Clone)]
pub struct ExpertEntry {
    pub manifest: Manifest,
    pub enabled: bool,
}

#[derive(Debug, Default)]
pub struct ExpertRegistry {
    experts: std::collections::HashMap<String, ExpertEntry>,
    active: Option<String>,
}

impl ExpertRegistry {
    /// 从扫描到的 expert manifests 装配；settings.experts 中 value=true 的专家为当前激活。
    pub fn new(manifests: Vec<Manifest>, settings: &PluginSettings) -> Self {
        let mut reg = Self::default();
        for m in manifests {
            let id = m.id.clone();
            let enabled = settings.experts.contains_key(&id);
            reg.experts.insert(
                id,
                ExpertEntry {
                    manifest: m,
                    enabled,
                },
            );
        }
        // active 特判：第一个 value=true 的专家。
        if let Some((id, true)) = settings.experts.iter().find(|(_, v)| **v) {
            if reg.experts.contains_key(id) {
                reg.active = Some(id.clone());
                if let Some(e) = reg.experts.get_mut(id) {
                    e.enabled = true;
                }
            }
        }
        reg
    }

    /// 激活专家：不存在 → 错误；存在 → 置 active + enabled。
    pub fn activate(&mut self, id: &str) -> Result<(), String> {
        if !self.experts.contains_key(id) {
            return Err(format!("未知专家 {id}"));
        }
        self.active = Some(id.to_string());
        if let Some(e) = self.experts.get_mut(id) {
            e.enabled = true;
        }
        Ok(())
    }

    /// 停用当前专家。
    pub fn deactivate(&mut self) {
        self.active = None;
    }

    pub fn active_expert(&self) -> Option<&ExpertEntry> {
        self.active
            .as_ref()
            .and_then(|id| self.experts.get(id))
    }

    pub fn active_id(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn list(&self) -> impl Iterator<Item = &ExpertEntry> {
        self.experts.values()
    }

    /// 专家提示词装配：base + 专家 system_prompt 追加段。
    pub fn expert_prompt(&self, base: &str) -> String {
        match self.active_expert() {
            Some(e) => {
                let extra = e.manifest.system_prompt.as_deref().unwrap_or("");
                if extra.trim().is_empty() {
                    base.to_string()
                } else {
                    format!("{base}\n\n{extra}")
                }
            }
            None => base.to_string(),
        }
    }

    /// 技能白名单（空 = 全开放）。
    pub fn whitelist(&self) -> Option<&[String]> {
        let e = self.active_expert()?;
        if e.manifest.skills.is_empty() {
            None
        } else {
            Some(&e.manifest.skills)
        }
    }

    pub fn persona_binding(&self) -> Option<&PersonaBinding> {
        let e = self.active_expert()?;
        let b = e.manifest.persona_binding.as_ref()?;
        if b.voice.is_none() && b.mood.is_none() {
            None
        } else {
            Some(b)
        }
    }
}

/// voice 联动（契约 §3.4）：已知 persona id → voice_profile 标签；未知 → 原样字符串。
pub fn voice_label(voice: &str) -> String {
    let known = ["gudian", "dushi", "wenrou", "huopo", "reqing"];
    if known.contains(&voice) {
        let profile = crate::voice_profile::default_profile_for(voice);
        crate::voice_profile::voice_tag(voice, &profile)
    } else {
        voice.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scientist_manifest() -> Manifest {
        serde_json::from_value(serde_json::json!({
            "id": "com.qibai.scientist",
            "type": "expert",
            "name": "科学家助手",
            "description": "科学家人设",
            "version": "1.0.0",
            "system_prompt": "你是一位严谨的科学家，说话简洁专业。",
            "skills": ["com.qibai.launch_app", "com.qibai.open_folder"],
            "persona_binding": {"voice": "gudian", "mood": "professional"}
        }))
        .unwrap()
    }

    #[test]
    fn activate_deactivate_and_prompt_assembly() {
        let mut reg = ExpertRegistry::new(vec![scientist_manifest()], &PluginSettings::default());
        assert!(reg.active_expert().is_none());
        // 未知 id → 错误
        assert!(reg.activate("com.nope").is_err());
        reg.activate("com.qibai.scientist").unwrap();
        assert!(reg.active_expert().unwrap().enabled);
        // prompt 追加装配
        let out = reg.expert_prompt("BASE");
        assert!(out.starts_with("BASE"));
        assert!(out.contains("严谨的科学家"));
        // 白名单
        let wl = reg.whitelist().unwrap();
        assert_eq!(wl.len(), 2);
        // 停用
        reg.deactivate();
        assert!(reg.active_expert().is_none());
        assert_eq!(reg.expert_prompt("BASE"), "BASE");
    }

    #[test]
    fn active_restored_from_settings() {
        let mut ps = PluginSettings::default();
        ps.experts.insert("com.qibai.scientist".into(), true);
        let reg = ExpertRegistry::new(vec![scientist_manifest()], &ps);
        assert_eq!(reg.active_id(), Some("com.qibai.scientist"));
        assert!(reg.persona_binding().is_some());
    }

    #[test]
    fn voice_linkage_known_vs_unknown() {
        assert_eq!(voice_label("gudian"), "温婉 · 中速 · 微低");
        assert_eq!(voice_label("custom-voice-x"), "custom-voice-x");
    }
}
