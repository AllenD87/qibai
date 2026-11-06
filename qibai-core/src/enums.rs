//! 跨模块共享的枚举：表情与动作。严格遵循 IPC 契约枚举集合。

use serde::{Deserialize, Serialize};
use std::fmt;

/// 表情枚举（契约固定集合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Emotion {
    Smiling,
    Happy,
    Worried,
    Sleepy,
    Focused,
    Sad,
    Shy,
    Surprised,
}

impl Default for Emotion {
    fn default() -> Self {
        Emotion::Smiling
    }
}

impl fmt::Display for Emotion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 输出小写下划线风格，与 JSON 契约一致
        let s = match self {
            Emotion::Smiling => "smiling",
            Emotion::Happy => "happy",
            Emotion::Worried => "worried",
            Emotion::Sleepy => "sleepy",
            Emotion::Focused => "focused",
            Emotion::Sad => "sad",
            Emotion::Shy => "shy",
            Emotion::Surprised => "surprised",
        };
        write!(f, "{s}")
    }
}

impl Emotion {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "smiling" => Emotion::Smiling,
            "happy" => Emotion::Happy,
            "worried" => Emotion::Worried,
            "sleepy" => Emotion::Sleepy,
            "focused" => Emotion::Focused,
            "sad" => Emotion::Sad,
            "shy" => Emotion::Shy,
            "surprised" => Emotion::Surprised,
            _ => return None,
        })
    }
}

/// 动作枚举（契约固定集合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Idle,
    Wave,
    Nod,
    Shake,
    Stretch,
    HandsOnHips,
    Touch,
    /// 节日彩蛋（预留）。
    Celebrate,
}

impl Default for Action {
    fn default() -> Self {
        Action::Idle
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Action::Idle => "idle",
            Action::Wave => "wave",
            Action::Nod => "nod",
            Action::Shake => "shake",
            Action::Stretch => "stretch",
            Action::HandsOnHips => "hands_on_hips",
            Action::Touch => "touch",
            Action::Celebrate => "celebrate",
        };
        write!(f, "{s}")
    }
}

impl Action {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "idle" => Action::Idle,
            "wave" => Action::Wave,
            "nod" => Action::Nod,
            "shake" => Action::Shake,
            "stretch" => Action::Stretch,
            "hands_on_hips" => Action::HandsOnHips,
            "touch" => Action::Touch,
            "celebrate" => Action::Celebrate,
            _ => return None,
        })
    }
}
