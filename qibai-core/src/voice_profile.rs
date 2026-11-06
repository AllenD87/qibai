//! 迭代⑤：每角色专属音色模型与参数映射。
//!
//! - [`VoiceProfile`]：每角色一份音色档案（mode / pitch / rate / 本地 engine voice id /
//!   云端 speaker），随 `personas.json` 持久化（`#[serde(default)]` 平滑迁移旧文件）。
//! - [`default_profile_for`]：Auto 模式下按 persona_id 映射的冻结默认音色表
//!   （云端 speaker 已按火山引擎官方音色列表核实）。
//! - [`resolve_profile`]：Custom → 存值 sanitize；Auto → 性格表默认值。
//! - [`voice_tag`]：前端状态栏展示用中文音色标签（与前端 JS 同款算法）。
//!
//! 隐私：本模块不接触 key；云端 speaker 仅作为 TTS body 字段，由 tts.rs 负责组装。

use crate::persona::personality_of;
use crate::tts::{RATE_MAX, RATE_MIN};
use serde::{Deserialize, Serialize};

/// pitch 落盘/使用前 clamp 区间。
pub const PITCH_MIN: f32 = -10.0;
pub const PITCH_MAX: f32 = 10.0;

/// 音色模式：Auto = 按性格表自动映射；Custom = 用户手动覆盖。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceProfileMode {
    Auto,
    Custom,
}

/// 每角色音色档案（serde 字段名与前端逐字段一致）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoiceProfile {
    pub mode: VoiceProfileMode,
    pub pitch: f32,
    pub rate: f32,
    pub engine_voice_id: String,
    pub cloud_speaker: String,
}

impl Default for VoiceProfile {
    fn default() -> Self {
        Self {
            mode: VoiceProfileMode::Auto,
            pitch: 0.0,
            rate: 1.0,
            engine_voice_id: String::new(),
            cloud_speaker: String::new(),
        }
    }
}

impl VoiceProfile {
    /// 落盘/使用前清洗：pitch clamp [-10,10]，rate clamp [0.5,2.0]，其余原样。
    pub fn sanitize(&self) -> VoiceProfile {
        let mut v = self.clone();
        v.pitch = v.pitch.clamp(PITCH_MIN, PITCH_MAX);
        v.rate = v.rate.clamp(RATE_MIN, RATE_MAX);
        v
    }
}

/// 按 persona_id 取 Auto 模式冻结默认音色表（未知 id 回退 gudian）。
/// 冻结值，已按火山引擎官方音色列表核实，勿改。
pub fn default_profile_for(persona_id: &str) -> VoiceProfile {
    let (pitch, rate, cloud_speaker) = match persona_id {
        // 温柔小雅
        "gudian" => (-2.0, 1.0, "zh_female_wenrouxiaoya_moon_bigtts"),
        // 爽快思思
        "dushi" => (0.0, 1.1, "zh_female_shuangkuaisisi_emo_v2_mars_bigtts"),
        // 软萌团子
        "wenrou" => (-1.0, 0.8, "ICL_zh_female_ruanmengtuanzi_cs_tob"),
        // 元气甜妹（wuxi）
        "huopo" => (3.0, 1.2, "ICL_zh_female_wuxi_tob"),
        // 热情艾娜
        "reqing" => (2.0, 1.05, "ICL_zh_female_reqingaina_cs_tob"),
        // 未知 id 回退 gudian
        _ => (-2.0, 1.0, "zh_female_wenrouxiaoya_moon_bigtts"),
    };
    VoiceProfile {
        mode: VoiceProfileMode::Auto,
        pitch,
        rate,
        engine_voice_id: String::new(),
        cloud_speaker: cloud_speaker.to_string(),
    }
}

/// 解析生效音色：Custom → 存值 sanitize；Auto → 性格表默认值。
pub fn resolve_profile(profile: &VoiceProfile, persona_id: &str) -> VoiceProfile {
    match profile.mode {
        VoiceProfileMode::Custom => profile.sanitize(),
        VoiceProfileMode::Auto => default_profile_for(persona_id),
    }
}

/// 语速描述（与前端 JS 同款算法；rate==1.1 归「轻快」，以 5 角色冻结标签为准）。
fn rate_desc(rate: f32) -> &'static str {
    if rate < 0.9 {
        "慢速"
    } else if rate < 1.1 {
        "中速"
    } else {
        "轻快"
    }
}

/// 音高描述。
fn pitch_desc(pitch: f32) -> &'static str {
    if pitch < -0.5 {
        "微低"
    } else if pitch <= 0.5 {
        "中性"
    } else if pitch <= 1.5 {
        "明亮"
    } else {
        "偏高"
    }
}

/// 中文音色标签（前端状态栏展示，与前端 JS 同款算法）：
/// Auto → "{tone} · {rate_desc} · {pitch_desc}"；Custom → "自定义 · {rate_desc} · {pitch_desc}"。
pub fn voice_tag(persona_id: &str, resolved: &VoiceProfile) -> String {
    let rate_d = rate_desc(resolved.rate);
    let pitch_d = pitch_desc(resolved.pitch);
    match resolved.mode {
        VoiceProfileMode::Custom => format!("自定义 · {rate_d} · {pitch_d}"),
        VoiceProfileMode::Auto => {
            let tone = personality_of(persona_id).tone;
            format!("{tone} · {rate_d} · {pitch_d}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_profile_serde_modes_and_defaults() {
        // "auto" / "custom" 两种模式字符串均可解析（snake_case）。
        let auto: VoiceProfile = serde_json::from_str(
            r#"{"mode":"auto","pitch":0.0,"rate":1.0,"engine_voice_id":"","cloud_speaker":""}"#,
        )
        .unwrap();
        assert_eq!(auto.mode, VoiceProfileMode::Auto);
        let custom: VoiceProfile = serde_json::from_str(
            r#"{"mode":"custom","pitch":3.0,"rate":1.2,"engine_voice_id":"id1","cloud_speaker":"spk"}"#,
        )
        .unwrap();
        assert_eq!(custom.mode, VoiceProfileMode::Custom);
        assert_eq!(custom.pitch, 3.0);
        // round-trip
        let s = serde_json::to_string(&custom).unwrap();
        let back: VoiceProfile = serde_json::from_str(&s).unwrap();
        assert_eq!(custom, back);
        // 默认值
        let d = VoiceProfile::default();
        assert_eq!(d.mode, VoiceProfileMode::Auto);
        assert_eq!(d.pitch, 0.0);
        assert_eq!(d.rate, 1.0);
        assert!(d.engine_voice_id.is_empty());
        assert!(d.cloud_speaker.is_empty());
    }

    #[test]
    fn sanitize_clamps_pitch_and_rate() {
        let mut v = VoiceProfile::default();
        v.pitch = 99.0;
        v.rate = 5.0;
        let s = v.sanitize();
        assert_eq!(s.pitch, PITCH_MAX); // 10.0
        assert_eq!(s.rate, RATE_MAX); // 2.0

        let mut v = VoiceProfile::default();
        v.pitch = -99.0;
        v.rate = 0.01;
        let s = v.sanitize();
        assert_eq!(s.pitch, PITCH_MIN); // -10.0
        assert_eq!(s.rate, RATE_MIN); // 0.5

        // 区间内原值不变；其余字段原样
        let mut v = VoiceProfile::default();
        v.mode = VoiceProfileMode::Custom;
        v.engine_voice_id = "x".into();
        v.cloud_speaker = "spk".into();
        v.pitch = 2.5;
        v.rate = 0.9;
        let s = v.sanitize();
        assert_eq!(s.pitch, 2.5);
        assert_eq!(s.rate, 0.9);
        assert_eq!(s.engine_voice_id, "x");
        assert_eq!(s.cloud_speaker, "spk");
    }

    #[test]
    fn default_profile_frozen_values_for_five_personas() {
        let g = default_profile_for("gudian");
        assert_eq!(g.pitch, -2.0);
        assert_eq!(g.rate, 1.0);
        assert_eq!(g.engine_voice_id, "");
        assert_eq!(g.cloud_speaker, "zh_female_wenrouxiaoya_moon_bigtts");

        let d = default_profile_for("dushi");
        assert_eq!(d.pitch, 0.0);
        assert_eq!(d.rate, 1.1);
        assert_eq!(d.cloud_speaker, "zh_female_shuangkuaisisi_emo_v2_mars_bigtts");

        let w = default_profile_for("wenrou");
        assert_eq!(w.pitch, -1.0);
        assert_eq!(w.rate, 0.8);
        assert_eq!(w.cloud_speaker, "ICL_zh_female_ruanmengtuanzi_cs_tob");

        let h = default_profile_for("huopo");
        assert_eq!(h.pitch, 3.0);
        assert_eq!(h.rate, 1.2);
        assert_eq!(h.cloud_speaker, "ICL_zh_female_wuxi_tob");

        let r = default_profile_for("reqing");
        assert_eq!(r.pitch, 2.0);
        assert_eq!(r.rate, 1.05);
        assert_eq!(r.cloud_speaker, "ICL_zh_female_reqingaina_cs_tob");

        // 未知 id 回退 gudian
        let u = default_profile_for("nosuch");
        assert_eq!(u, g);
    }

    #[test]
    fn resolve_profile_auto_vs_custom() {
        // Auto → 性格表默认值（忽略存值）
        let stored = VoiceProfile {
            mode: VoiceProfileMode::Auto,
            pitch: 99.0,
            rate: 99.0,
            engine_voice_id: "ignored".into(),
            cloud_speaker: "ignored".into(),
        };
        let r = resolve_profile(&stored, "huopo");
        assert_eq!(r, default_profile_for("huopo"));

        // Custom → 存值 sanitize
        let custom = VoiceProfile {
            mode: VoiceProfileMode::Custom,
            pitch: 50.0,
            rate: 5.0,
            engine_voice_id: "my-voice".into(),
            cloud_speaker: "my-speaker".into(),
        };
        let r = resolve_profile(&custom, "gudian");
        assert_eq!(r.mode, VoiceProfileMode::Custom);
        assert_eq!(r.pitch, PITCH_MAX);
        assert_eq!(r.rate, RATE_MAX);
        assert_eq!(r.engine_voice_id, "my-voice");
        assert_eq!(r.cloud_speaker, "my-speaker");
    }

    #[test]
    fn voice_tag_frozen_five_labels() {
        assert_eq!(voice_tag("gudian", &default_profile_for("gudian")), "温婉 · 中速 · 微低");
        assert_eq!(voice_tag("dushi", &default_profile_for("dushi")), "知性 · 轻快 · 中性");
        assert_eq!(voice_tag("wenrou", &default_profile_for("wenrou")), "恬静 · 慢速 · 微低");
        assert_eq!(voice_tag("huopo", &default_profile_for("huopo")), "元气 · 轻快 · 偏高");
        assert_eq!(voice_tag("reqing", &default_profile_for("reqing")), "热情 · 中速 · 偏高");
        // 未知 id 回退 gudian tone
        assert_eq!(voice_tag("nosuch", &default_profile_for("nosuch")), "温婉 · 中速 · 微低");
        // Custom → 「自定义 · …」
        let custom = VoiceProfile {
            mode: VoiceProfileMode::Custom,
            pitch: 3.0,
            rate: 0.7,
            ..VoiceProfile::default()
        };
        assert_eq!(voice_tag("gudian", &custom), "自定义 · 慢速 · 偏高");
    }
}
