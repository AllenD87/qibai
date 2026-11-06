//! 提醒调度：久坐 / 喝水 / 护眼。
//!
//! - 纯逻辑部分（`due_kinds`）与时钟解耦，可在测试中注入可控时钟。
//! - 真实运行时用 tokio 定时循环，触发时：发系统通知（OS 层）+ 通过 EventSink
//!   向前端 emit `qibai://reminder` 事件（含气泡文案 + 动作）。

use crate::enums::Action;
use crate::settings::ReminderSettings;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderKind {
    Sit,
    Water,
    Eye,
}

impl ReminderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReminderKind::Sit => "sit",
            ReminderKind::Water => "water",
            ReminderKind::Eye => "eye",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "sit" => Some(ReminderKind::Sit),
            "water" => Some(ReminderKind::Water),
            "eye" => Some(ReminderKind::Eye),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReminderEvent {
    #[serde(rename = "type")]
    pub kind: ReminderKind,
    pub message: String,
    pub action: Action,
}

impl ReminderEvent {
    pub fn for_kind(kind: ReminderKind) -> Self {
        let (message, action) = match kind {
            ReminderKind::Sit => (
                "已经坐了很久啦，站起来活动一下吧～".to_string(),
                Action::Stretch,
            ),
            ReminderKind::Water => (
                "该喝水啦，补充水分才更有精神".to_string(),
                Action::Nod,
            ),
            ReminderKind::Eye => (
                "眼睛休息一下吧，看看远处".to_string(),
                Action::Shake,
            ),
        };
        Self {
            kind,
            message,
            action,
        }
    }
}

/// 时钟抽象：便于测试。
pub trait Clock: Send + Sync {
    fn now_secs(&self) -> u64;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now_secs(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// 提醒调度状态机。
pub struct ReminderScheduler<C: Clock = SystemClock> {
    settings: ReminderSettings,
    last_fired: std::collections::HashMap<ReminderKind, u64>,
    clock: C,
}

impl Default for ReminderScheduler<SystemClock> {
    fn default() -> Self {
        Self::new(ReminderSettings::default(), SystemClock)
    }
}

impl<C: Clock> ReminderScheduler<C> {
    pub fn new(settings: ReminderSettings, clock: C) -> Self {
        let now = clock.now_secs();
        let mut last_fired = std::collections::HashMap::new();
        // 启动即初始化基线，避免启动瞬间立刻触发
        last_fired.insert(ReminderKind::Sit, now);
        last_fired.insert(ReminderKind::Water, now);
        last_fired.insert(ReminderKind::Eye, now);
        Self {
            settings,
            last_fired,
            clock,
        }
    }

    pub fn update_settings(&mut self, settings: ReminderSettings) {
        self.settings = settings;
    }

    /// 计算当前到期的提醒（不改状态）。
    pub fn due_kinds(&self) -> Vec<ReminderKind> {
        let now = self.clock.now_secs();
        let mut out = Vec::new();
        let candidates = [
            (ReminderKind::Sit, self.settings.enabled.sit, self.settings.sit_min),
            (ReminderKind::Water, self.settings.enabled.water, self.settings.water_min),
            (ReminderKind::Eye, self.settings.enabled.eye, self.settings.eye_min),
        ];
        for (kind, enabled, minutes) in candidates {
            if !enabled || minutes == 0 {
                continue;
            }
            let last = self.last_fired.get(&kind).copied().unwrap_or(now);
            if now.saturating_sub(last) >= minutes as u64 * 60 {
                out.push(kind);
            }
        }
        out
    }

    /// 取出到期提醒并重置基线。
    pub fn take_due(&mut self) -> Vec<ReminderEvent> {
        let due = self.due_kinds();
        let now = self.clock.now_secs();
        due.iter().for_each(|k| {
            self.last_fired.insert(*k, now);
        });
        due.iter().map(|k| ReminderEvent::for_kind(*k)).collect()
    }

    /// 立即起算一次提醒计时（语音「提醒我喝水」用）：把该 kind 的 last_fired 置为
    /// now - (interval - delay)，使下一个 due 在 delay 秒后到来；delay 缺省=完整间隔。
    /// 返回距离下次触发的分钟数（供语音回复）。
    pub fn schedule_once(&mut self, kind: &ReminderKind, delay_secs: Option<u64>) -> u64 {
        let now = self.clock.now_secs();
        let minutes = match kind {
            ReminderKind::Sit => self.settings.sit_min,
            ReminderKind::Water => self.settings.water_min,
            ReminderKind::Eye => self.settings.eye_min,
        };
        let interval = minutes as u64 * 60;
        let delay = delay_secs.unwrap_or(interval).min(interval);
        let base = now.saturating_sub(interval.saturating_sub(delay));
        self.last_fired.insert(kind.clone(), base);
        // 让该 kind 启用（语音指令显式开启）
        match kind {
            ReminderKind::Sit => self.settings.enabled.sit = true,
            ReminderKind::Water => self.settings.enabled.water = true,
            ReminderKind::Eye => self.settings.enabled.eye = true,
        }
        ((delay as f64) / 60.0).round().max(1.0) as u64
    }

    /// 取消某类提醒（语音「取消喝水提醒」用）：停用该 kind（不清基线，恢复时重新起算）。
    pub fn cancel_kind(&mut self, kind: &ReminderKind) {
        match kind {
            ReminderKind::Sit => self.settings.enabled.sit = false,
            ReminderKind::Water => self.settings.enabled.water = false,
            ReminderKind::Eye => self.settings.enabled.eye = false,
        }
    }

    /// 当前各 kind 启用状态（schedule_once/cancel_kind 改动后同步回持久化 settings 用）。
    pub fn enabled_snapshot(&self) -> crate::settings::ReminderEnabled {
        self.settings.enabled.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeClock(u64);
    impl Clock for FakeClock {
        fn now_secs(&self) -> u64 {
            self.0
        }
    }

    fn settings() -> ReminderSettings {
        ReminderSettings {
            sit_min: 45,
            water_min: 30,
            eye_min: 20,
            enabled: crate::settings::ReminderEnabled {
                sit: true,
                water: true,
                eye: true,
            },
        }
    }

    #[test]
    fn fires_after_interval_elapsed() {
        let mut s = ReminderScheduler::new(settings(), FakeClock(1000));
        // 快进到 46 分钟后
        s.clock = FakeClock(1000 + 46 * 60);
        let due = s.take_due();
        // sit(45) 到期；water(30) 也到期；eye(20) 也到期
        assert!(due.iter().any(|e| e.kind == ReminderKind::Sit));
        assert!(due.iter().any(|e| e.kind == ReminderKind::Water));
        assert!(due.iter().any(|e| e.kind == ReminderKind::Eye));
    }

    #[test]
    fn disabled_reminder_never_fires() {
        let mut cfg = settings();
        cfg.enabled.water = false;
        let mut s = ReminderScheduler::new(cfg, FakeClock(1000));
        s.clock = FakeClock(1000 + 120 * 60);
        let due = s.take_due();
        assert!(!due.iter().any(|e| e.kind == ReminderKind::Water));
    }

    #[test]
    fn baseline_resets_after_take() {
        let mut s = ReminderScheduler::new(settings(), FakeClock(1000));
        s.clock = FakeClock(1000 + 31 * 60);
        let first = s.take_due();
        assert!(first.iter().any(|e| e.kind == ReminderKind::Water));
        // 再过 1 分钟：water 不应再次触发（基线已重置）
        s.clock = FakeClock(1000 + 32 * 60);
        let second = s.take_due();
        assert!(!second.iter().any(|e| e.kind == ReminderKind::Water));
    }

    #[test]
    fn event_shape_matches_contract() {
        let ev = ReminderEvent::for_kind(ReminderKind::Sit);
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "sit");
        assert!(v["message"].is_string());
        assert_eq!(v["action"], "stretch");
    }
}
