//! 全屏守护演出 —— 纯逻辑模块（无 GUI / Tauri 依赖）。
//!
//! 风格对齐 [`crate::reminder`]：状态机与时钟解耦，测试中可注入可控时钟。
//! 本模块**只依赖 serde / serde_json / thiserror 与自带类型**，不引用 crate 内其他模块，
//! 可在独立 scratch 工程中单独冒烟编译。
//!
//! 状态机：`Idle → Active ⇄(finish/skip) Done →(冷却结束) Idle`
//! - `Idle`：待命，可被久坐（`on_sit_due`）或伴侣主动（`manual_trigger`）触发；
//! - `Active`：演出进行中，可 `skip`（不执行收尾动作）或 `finish`（自然结束，执行收尾动作）；
//! - `Done`：进入 30 分钟冷却期，冷却期内任何触发一律返回 `None`。
//!
//! 前端接线（见 `docs/GUARD.md`）：
//! - 触发成功后由外层 emit `qibai://guard-start` 事件（payload 即 [`GuardStart`]）；
//! - 演出结束（自然 / 跳过）后由外层 emit `qibai://guard-end`；
//! - `sleep` / `lock` 收尾动作通过 `guard_system_action` 命令执行，命令字符串由
//!   [`platform_sleep_cmd`] / [`platform_lock_cmd`] 提供。

use serde::{Deserialize, Serialize};

/// 冷却时长：一次演出结束后 30 分钟内不再触发，防止反复打扰。
pub const COOLDOWN_SECS: u64 = 30 * 60;

/// 守护模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardMode {
    /// 轻量模式：任何来源都只发通知，不进行全屏演出。
    Light,
    /// 强力模式：全屏演出。
    Strong,
    /// 锁定模式：全屏演出，且收尾动作强制为锁屏。
    Lock,
}

impl GuardMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            GuardMode::Light => "light",
            GuardMode::Strong => "strong",
            GuardMode::Lock => "lock",
        }
    }
}

/// 演出结束后的收尾动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardEndAction {
    /// 不做任何事，演出直接结束。
    None,
    /// 息屏（显示器休眠）。
    Sleep,
    /// 锁定系统。
    Lock,
    /// 展示休息倒计时遮罩，倒计时结束后结束。
    Countdown,
}

impl GuardEndAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            GuardEndAction::None => "none",
            GuardEndAction::Sleep => "sleep",
            GuardEndAction::Lock => "lock",
            GuardEndAction::Countdown => "countdown",
        }
    }
}

fn default_enabled() -> bool {
    true
}
fn default_sit_min() -> u32 {
    120
}
fn default_mode() -> GuardMode {
    GuardMode::Strong
}
fn default_end_action() -> GuardEndAction {
    GuardEndAction::Countdown
}
fn default_countdown_secs() -> u32 {
    120
}

/// 倒计时合法区间：60..=300 秒。
pub const COUNTDOWN_MIN: u32 = 60;
pub const COUNTDOWN_MAX: u32 = 300;

/// 守护设置。全部字段带 serde default；反序列化后建议调用 [`GuardSettings::sanitize`]
/// 规整（`countdown_secs` clamp 到 60..=300），调度器在 new / update_settings 内部已自动规整。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardSettings {
    /// 总开关：false 时任何触发都返回 None。
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 连续工作多少分钟后触发（与久坐提醒对齐）。
    #[serde(default = "default_sit_min")]
    pub sit_min: u32,
    #[serde(default = "default_mode")]
    pub mode: GuardMode,
    #[serde(default = "default_end_action")]
    pub end_action: GuardEndAction,
    /// 休息倒计时秒数，会被 clamp 到 60..=300。
    #[serde(default = "default_countdown_secs")]
    pub countdown_secs: u32,
}

impl Default for GuardSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            sit_min: 120,
            mode: GuardMode::Strong,
            end_action: GuardEndAction::Countdown,
            countdown_secs: 120,
        }
    }
}

impl GuardSettings {
    /// 规整非法取值：`countdown_secs` clamp 到 60..=300。
    pub fn sanitize(mut self) -> Self {
        self.countdown_secs = self.countdown_secs.clamp(COUNTDOWN_MIN, COUNTDOWN_MAX);
        self
    }
}

/// 时钟抽象：便于测试注入可控时钟（对齐 reminder 模块风格）。
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

/// 一次守护演出的启动指令（外层据此 emit `qibai://guard-start` 事件）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardStart {
    pub mode: GuardMode,
    /// 换算后的连续工作小时数：`max(1, sit_min / 60)`。
    pub hours: u32,
    /// 全龄友好提醒文案，如「你连续工作 2 小时啦，休息五分钟吧」。
    pub message: String,
    pub end_action: GuardEndAction,
    /// 已 clamp 到 60..=300。
    pub countdown_secs: u32,
    /// Light 模式恒为 false（仅通知，不演出）；Strong / Lock 为 true。
    pub perform_fullscreen: bool,
}

/// 调度器错误。
#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    /// 当前没有进行中的演出，不能 skip / finish。
    #[error("guard is not active")]
    NotActive,
}

/// 调度器内部状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// 待命（含冷却结束后的回归）。
    Idle,
    /// 演出进行中，记录开始时刻。
    Active { started_at: u64 },
    /// 已结束，记录结束时刻用于冷却计算。
    Done { ended_at: u64 },
}

/// 守护调度状态机。
pub struct GuardScheduler<C: Clock = SystemClock> {
    settings: GuardSettings,
    clock: C,
    state: State,
}

impl Default for GuardScheduler<SystemClock> {
    fn default() -> Self {
        Self::new(GuardSettings::default(), SystemClock)
    }
}

impl<C: Clock> GuardScheduler<C> {
    pub fn new(settings: GuardSettings, clock: C) -> Self {
        Self {
            settings: settings.sanitize(),
            clock,
            state: State::Idle,
        }
    }

    pub fn update_settings(&mut self, settings: GuardSettings) {
        self.settings = settings.sanitize();
    }

    /// 读取当前时钟秒数（便捷方法；触发类接口统一由调用方传入 now，便于测试注入）。
    pub fn now(&self) -> u64 {
        self.clock.now_secs()
    }

    /// 演出是否进行中。
    pub fn is_active(&self) -> bool {
        matches!(self.state, State::Active { .. })
    }

    /// 剩余冷却秒数；不在冷却期则为 0。
    pub fn cooldown_left(&self, now: u64) -> u64 {
        match self.state {
            State::Done { ended_at } => {
                let elapsed = now.saturating_sub(ended_at);
                if elapsed >= COOLDOWN_SECS {
                    // 冷却结束：回归待命态。
                    0
                } else {
                    COOLDOWN_SECS - elapsed
                }
            }
            _ => 0,
        }
    }

    /// 内部：尝试触发一次演出。返回 None 的情形：未启用 / 正在演出 / 冷却期内。
    fn try_trigger(&mut self, now: u64) -> Option<GuardStart> {
        if !self.settings.enabled {
            return None;
        }
        match self.state {
            State::Active { .. } => return None,
            State::Done { ended_at } => {
                if now.saturating_sub(ended_at) < COOLDOWN_SECS {
                    return None;
                }
                // 冷却已过，继续往下走，状态在下方置为 Active。
            }
            State::Idle => {}
        }

        let start = self.build_start();
        self.state = State::Active { started_at: now };
        Some(start)
    }

    /// 久坐到期入口（由久坐提醒 / 定时巡检调用）。
    pub fn on_sit_due(&mut self, now: u64) -> Option<GuardStart> {
        self.try_trigger(now)
    }

    /// 伴侣主动触发入口（伴侣/用户从 UI 手动发起）。
    pub fn manual_trigger(&mut self, now: u64) -> Option<GuardStart> {
        self.try_trigger(now)
    }

    /// 演出进行中被跳过（用户点跳过 / ESC）。跳过不执行收尾动作。
    /// 不在演出中返回 [`GuardError::NotActive`]。
    pub fn skip(&mut self, now: u64, reason: &str) -> Result<(), GuardError> {
        match self.state {
            State::Active { .. } => {
                // reason 透传给前端日志（'user' | 'auto'）；纯逻辑层仅记录语义，
                // 关键语义是：skip 之后 Done 态开启冷却，收尾动作由前端跳过。
                let _ = reason;
                self.state = State::Done { ended_at: now };
                Ok(())
            }
            _ => Err(GuardError::NotActive),
        }
    }

    /// 演出自然结束（含收尾动作执行完毕）。开启冷却。
    pub fn finish(&mut self, now: u64) -> Result<(), GuardError> {
        match self.state {
            State::Active { .. } => {
                self.state = State::Done { ended_at: now };
                Ok(())
            }
            _ => Err(GuardError::NotActive),
        }
    }

    /// 按当前设置构造演出指令（路由规则在此集中）。
    fn build_start(&self) -> GuardStart {
        let hours = (self.settings.sit_min / 60).max(1);
        let mode = self.settings.mode;
        // Lock 模式：收尾动作强制为 Lock。
        let end_action = match mode {
            GuardMode::Lock => GuardEndAction::Lock,
            _ => self.settings.end_action,
        };
        // Light 模式：任何来源都不进行全屏演出（仅通知）。
        let perform_fullscreen = match mode {
            GuardMode::Light => false,
            GuardMode::Strong | GuardMode::Lock => true,
        };
        GuardStart {
            mode,
            hours,
            message: format!("你连续工作 {} 小时啦，休息五分钟吧", hours),
            end_action,
            countdown_secs: self.settings.countdown_secs,
            perform_fullscreen,
        }
    }
}

/// 平台动作层：返回对应 OS 的命令字符串（供外层 spawn 执行）。
///
/// 真机验证项：以下命令在当前沙箱无桌面 GUI，均需在真机上实测后再接入自动执行。
/// ── Linux ──────────────────────────────────────────────────────
#[cfg(target_os = "linux")]
/// 真机验证项：息屏（DPMS 关闭显示器）。
pub fn platform_sleep_cmd() -> &'static str {
    "xset dpms force off"
}

#[cfg(target_os = "linux")]
/// 真机验证项：锁屏。
pub fn platform_lock_cmd() -> &'static str {
    "loginctl lock-session"
}

/// ── Windows ────────────────────────────────────────────────────
#[cfg(target_os = "windows")]
/// 真机验证项：系统休眠。
pub fn platform_sleep_cmd() -> &'static str {
    "rundll32.exe powrprof.dll,SetSuspendState 0,1,0"
}

#[cfg(target_os = "windows")]
/// 真机验证项：锁定工作站。
pub fn platform_lock_cmd() -> &'static str {
    "rundll32.exe user32.dll,LockWorkStation"
}

/// ── macOS ──────────────────────────────────────────────────────
#[cfg(target_os = "macos")]
/// 真机验证项：关闭显示器。
pub fn platform_sleep_cmd() -> &'static str {
    "pmset displaysleepnow"
}

#[cfg(target_os = "macos")]
/// 真机验证项：锁屏（候补方案 —— 模拟 Ctrl+Cmd+Q 启动屏保）。
/// 需要在「系统设置 → 隐私与安全性 → 辅助功能」中为宿主进程授权，否则按键码会失败。
pub fn platform_lock_cmd() -> &'static str {
    "osascript -e 'tell application \"System Events\" to keystroke \"q\" using {command down, control down}'"
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

    fn settings(mut s: GuardSettings) -> GuardSettings {
        // 模拟反序列化入口：与生产路径一样先 sanitize。
        s = s.sanitize();
        s
    }

    /// 完整周期：Idle → Active → Done → 冷却。
    #[test]
    fn full_lifecycle_routes_idle_to_done_with_cooldown() {
        let mut s = GuardScheduler::new(settings(GuardSettings::default()), FakeClock(1000));
        assert!(!s.is_active());

        let start = s.on_sit_due(1000).expect("should fire at t=1000");
        assert!(s.is_active());
        assert_eq!(start.perform_fullscreen, true); // Strong 默认
        assert_eq!(start.mode, GuardMode::Strong);

        // 演出期间再次触发 → None（防重入）
        assert!(s.on_sit_due(1001).is_none());
        assert!(s.manual_trigger(1001).is_none());

        s.finish(1200).unwrap();
        assert!(!s.is_active());
        // 冷却期内：立即触发一律 None
        assert_eq!(s.cooldown_left(1200), COOLDOWN_SECS);
        assert!(s.on_sit_due(1200).is_none());
        // 快进 31 分钟后：冷却结束，可再次触发
        assert!(s.on_sit_due(1200 + COOLDOWN_SECS + 60).is_some());
    }

    /// skip 中断：演出中可跳过，跳过后进冷却；且不执行收尾动作。
    #[test]
    fn skip_interrupts_and_starts_cooldown() {
        let mut s = GuardScheduler::new(settings(GuardSettings::default()), FakeClock(0));
        assert!(s.manual_trigger(0).is_some());
        s.skip(60, "user").unwrap();
        assert!(!s.is_active());
        // skip 后同样进入冷却
        assert_eq!(s.cooldown_left(60), COOLDOWN_SECS);
        assert!(s.on_sit_due(61).is_none());
        // 未激活时再 skip / finish → NotActive
        assert!(matches!(s.skip(62, "auto"), Err(GuardError::NotActive)));
        assert!(matches!(s.finish(62), Err(GuardError::NotActive)));
    }

    /// 冷却防重：冷却窗口内任何来源（久坐 / 手动）都返回 None，出窗口恢复。
    #[test]
    fn cooldown_blocks_any_source_until_expired() {
        let mut s = GuardScheduler::new(settings(GuardSettings::default()), FakeClock(0));
        s.manual_trigger(0).unwrap();
        s.finish(0).unwrap();
        // 冷却剩 1 秒时仍阻挡
        let t = COOLDOWN_SECS - 1;
        assert_eq!(s.cooldown_left(t), 1);
        assert!(s.on_sit_due(t).is_none());
        assert!(s.manual_trigger(t).is_none());
        // 刚满冷却窗口：放行
        assert!(s.manual_trigger(COOLDOWN_SECS).is_some());
    }

    /// Light 模式：任何来源都不演出（perform_fullscreen = false）。
    #[test]
    fn light_mode_never_performs_fullscreen() {
        let mut cfg = GuardSettings::default();
        cfg.mode = GuardMode::Light;
        let mut s = GuardScheduler::new(settings(cfg), FakeClock(0));
        let start = s.manual_trigger(0).expect("light still emits a start");
        assert_eq!(start.perform_fullscreen, false);
        // 久坐来源同样不演出：skip 后进冷却，出冷却再由久坐入口触发
        s.skip(1, "user").unwrap();
        let again = s.on_sit_due(COOLDOWN_SECS + 1).unwrap();
        assert_eq!(again.perform_fullscreen, false);
    }

    /// Lock 模式：收尾动作强制为 Lock（即便设置里是 Countdown）。
    #[test]
    fn lock_mode_forces_lock_end_action() {
        let mut cfg = GuardSettings::default();
        cfg.mode = GuardMode::Lock;
        cfg.end_action = GuardEndAction::Countdown;
        let mut s = GuardScheduler::new(settings(cfg), FakeClock(0));
        let start = s.manual_trigger(0).unwrap();
        assert_eq!(start.mode, GuardMode::Lock);
        assert_eq!(start.end_action, GuardEndAction::Lock);
        assert_eq!(start.perform_fullscreen, true);
    }

    /// hours 换算：sit_min/60 向下取整，最小为 1。
    #[test]
    fn hours_conversion_floors_and_floors_at_one() {
        let mk = |sit_min: u32| -> u32 {
            let mut cfg = GuardSettings::default();
            cfg.sit_min = sit_min;
            let mut s = GuardScheduler::new(settings(cfg), FakeClock(0));
            s.manual_trigger(0).unwrap().hours
        };
        assert_eq!(mk(120), 2); // 2 小时
        assert_eq!(mk(90), 1);  // 1.5h 向下取整
        assert_eq!(mk(30), 1);  // 不足 1 小时 → 兜底为 1
        assert_eq!(mk(300), 5);
    }

    /// 消息文案：包含工作小时数，全龄友好。
    #[test]
    fn message_text_contains_hours_and_is_family_friendly() {
        let mut cfg = GuardSettings::default();
        cfg.sit_min = 120;
        let mut s = GuardScheduler::new(settings(cfg), FakeClock(0));
        let start = s.manual_trigger(0).unwrap();
        assert_eq!(start.message, "你连续工作 2 小时啦，休息五分钟吧");
        assert!(start.message.contains("休息五分钟"));
    }

    /// countdown_secs clamp：低于 60 提到 60，高于 300 压到 300。
    #[test]
    fn countdown_secs_is_clamped() {
        let mk = |secs: u32| -> u32 {
            let mut cfg = GuardSettings::default();
            cfg.countdown_secs = secs;
            let mut s = GuardScheduler::new(settings(cfg), FakeClock(0));
            s.manual_trigger(0).unwrap().countdown_secs
        };
        assert_eq!(mk(10), 60);
        assert_eq!(mk(9999), 300);
        assert_eq!(mk(120), 120);
        assert_eq!(mk(60), 60);
        assert_eq!(mk(300), 300);
    }

    /// 总开关关闭：任何触发都返回 None。
    #[test]
    fn disabled_settings_never_fire() {
        let mut cfg = GuardSettings::default();
        cfg.enabled = false;
        let mut s = GuardScheduler::new(settings(cfg), FakeClock(0));
        assert!(s.on_sit_due(0).is_none());
        assert!(s.manual_trigger(0).is_none());
    }

    /// serde 形状：枚举 snake_case，字段名与前端 payload 契约对齐。
    #[test]
    fn serde_shape_matches_frontend_contract() {
        let cfg = GuardSettings::default();
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["enabled"], true);
        assert_eq!(v["sit_min"], 120);
        assert_eq!(v["mode"], "strong");
        assert_eq!(v["end_action"], "countdown");
        assert_eq!(v["countdown_secs"], 120);

        let mut s = GuardScheduler::new(settings(cfg), FakeClock(0));
        let start = s.manual_trigger(0).unwrap();
        let v = serde_json::to_value(&start).unwrap();
        assert_eq!(v["mode"], "strong");
        assert_eq!(v["hours"], 2);
        assert_eq!(v["end_action"], "countdown");
        assert_eq!(v["countdown_secs"], 120);
        assert_eq!(v["perform_fullscreen"], true);
        assert!(v["message"].is_string());
    }

    /// 平台动作层：编译期按目标 OS 返回字符串（本沙箱为 linux，仅冒烟调用）。
    #[test]
    fn platform_cmds_are_nonempty_strings() {
        assert!(!platform_sleep_cmd().is_empty());
        assert!(!platform_lock_cmd().is_empty());
    }
}
