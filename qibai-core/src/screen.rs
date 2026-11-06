//! 屏幕「眼睛 + 手」核心逻辑（v18.3）：截图工具探测、坐标换算、GUI 分级授权。
//!
//! 安全原则：
//! - 屏幕内容只做「理解/展示」，操作只由用户语音指令或面板主动点击触发（防 Prompt 注入隔离）；
//! - GUI 操作（点击/输入/按键）受 [`GuiGuard`] 分级授权：截图低危直接可用，
//!   键鼠操作需开启开关；每次会话可临时授权（面板主动点击即显式授权一次）；
//! - 进程执行放 src-tauri 命令层，本模块只做可单测的纯逻辑。

use serde::{Deserialize, Serialize};

/// 截图工具优先级（X11 优先；grim 用于 Wayland）。
pub const CAPTURE_TOOLS: &[&str] = &["import", "gnome-screenshot", "scrot", "xwd", "grim"];
/// GUI 操作依赖。
pub const GUI_TOOLS: &[&str] = &["xdotool", "wmctrl"];

/// 构造某工具的截图命令 argv（输出到指定 png 路径）。
/// 返回 None 表示该工具不受支持（未探测到/参数不适用）。
pub fn capture_argv(tool: &str, out: &str) -> Option<Vec<String>> {
    match tool {
        // ImageMagick: import -window root out.png（全屏）
        "import" => Some(vec![
            "import".into(),
            "-window".into(),
            "root".into(),
            out.into(),
        ]),
        // gnome-screenshot -f out.png（全屏）
        "gnome-screenshot" => Some(vec!["gnome-screenshot".into(), "-f".into(), out.into()]),
        // scrot out.png
        "scrot" => Some(vec!["scrot".into(), out.into()]),
        // xwd -root -out out.xwd
        "xwd" => Some(vec!["xwd".into(), "-root".into(), "-out".into(), out.into()]),
        // grim out.png（Wayland）
        "grim" => Some(vec!["grim".into(), out.into()]),
        _ => None,
    }
}

/// 面板显示坐标 → 屏幕物理坐标换算（按显示尺寸与原图像素尺寸比例）。
/// 返回 (screen_x, screen_y)，越界输入夹回合法范围。
pub fn scale_coords(
    native_w: u32,
    native_h: u32,
    disp_w: f64,
    disp_h: f64,
    x: f64,
    y: f64,
) -> (u32, u32) {
    if native_w == 0 || native_h == 0 || disp_w <= 0.0 || disp_h <= 0.0 {
        return (0, 0);
    }
    let sx = (x * native_w as f64 / disp_w).round().clamp(0.0, native_w as f64 - 1.0);
    let sy = (y * native_h as f64 / disp_h).round().clamp(0.0, native_h as f64 - 1.0);
    (sx as u32, sy as u32)
}

/// 输入文本安全校验：拒绝不可打印控制字符与过长文本（xdotool type 防注入）。
pub fn sanitize_input_text(text: &str) -> Result<String, &'static str> {
    let t = text.trim();
    if t.is_empty() {
        return Err("输入内容为空");
    }
    if t.chars().count() > 200 {
        return Err("输入内容过长（限 200 字符）");
    }
    // 拒绝控制字符（防终端/编辑器注入）
    if t.chars().any(|c| c.is_control()) {
        return Err("输入内容含控制字符，已拒绝");
    }
    Ok(t.to_string())
}

/// 键名安全校验：仅允许已知白名单键（xdotool key 防注入）。
pub fn sanitize_key_name(keys: &str) -> Result<String, &'static str> {
    const KEYS: &[&str] = &[
        "Return", "Escape", "Tab", "BackSpace", "Delete", "space", "Up", "Down", "Left", "Right",
        "Home", "End", "Page_Up", "Page_Down", "ctrl+s", "ctrl+c", "ctrl+v", "ctrl+a", "ctrl+z",
        "ctrl+y", "ctrl+f", "ctrl+w", "ctrl+t", "ctrl+r", "alt+Tab", "ctrl+Tab", "ctrl+shift+Tab",
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
        "Super", "Menu", "Insert", "Print", "Pause",
    ];
    let k = keys.trim().to_ascii_lowercase();
    if KEYS.iter().any(|e| e.to_ascii_lowercase() == k) {
        Ok(keys.trim().to_string())
    } else {
        Err("不支持的键名（白名单外，已拒绝）")
    }
}

/// GUI 分级授权状态机。
/// - 截图（screen:capture）低危：`capture_allowed()` 恒为 true；
/// - 键鼠操作（点击/输入/按键）中高危：需 `enabled`（语音开关）或 `session_tokens`
///   （面板主动点击=显式授权一次，用后即焚）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiGuard {
    /// 语音/开关开启的持久授权（默认 false，符合本地优先、确认才动）。
    pub enabled: bool,
    /// 会话临时授权次数（面板主动点击/确认后 +1，消费后 -1）。
    pub session_tokens: u32,
}

impl Default for GuiGuard {
    fn default() -> Self {
        Self {
            enabled: false,
            session_tokens: 0,
        }
    }
}

impl GuiGuard {
    pub fn grant(&mut self) {
        self.enabled = true;
    }
    pub fn revoke(&mut self) {
        self.enabled = false;
        self.session_tokens = 0;
    }
    /// 面板主动点击等显式用户操作 → 临时授权一次。
    pub fn add_token(&mut self) {
        self.session_tokens = self.session_tokens.saturating_add(1);
    }
    /// 检查并消费一次授权（enabled 或 token）。返回是否允许。
    pub fn consume(&mut self) -> bool {
        if self.enabled {
            return true;
        }
        if self.session_tokens > 0 {
            self.session_tokens -= 1;
            return true;
        }
        false
    }
    pub fn is_granted(&self) -> bool {
        self.enabled || self.session_tokens > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_argv_supported() {
        let v = capture_argv("import", "/tmp/a.png").unwrap();
        assert_eq!(v, vec!["import", "-window", "root", "/tmp/a.png"]);
        assert!(capture_argv("scrot", "/tmp/a.png").is_some());
        assert!(capture_argv("nonexistent", "/tmp/a.png").is_none());
    }

    #[test]
    fn scale_coords_ratio() {
        // 4K 原图 (3840x2160) 显示在 960px 宽面板：中点换算回物理像素
        let (x, y) = scale_coords(3840, 2160, 960.0, 540.0, 480.0, 270.0);
        assert_eq!(x, 1920);
        assert_eq!(y, 1080);
    }

    #[test]
    fn scale_coords_clamp_and_guard() {
        let (x, y) = scale_coords(100, 100, 50.0, 50.0, 999.0, -5.0);
        assert_eq!(x, 99);
        assert_eq!(y, 0);
        let (x2, _) = scale_coords(0, 0, 50.0, 50.0, 10.0, 10.0);
        assert_eq!(x2, 0);
    }

    #[test]
    fn sanitize_input_rejects_control_and_long() {
        assert!(sanitize_input_text("hello 世界").is_ok());
        assert!(sanitize_input_text("\u{1b}[2J 恶意转义").is_err());
        assert!(sanitize_input_text(&"x".repeat(300)).is_err());
        assert!(sanitize_input_text("   ").is_err());
    }

    #[test]
    fn sanitize_key_whitelist() {
        assert!(sanitize_key_name("Return").is_ok());
        assert!(sanitize_key_name("ctrl+s").is_ok());
        assert!(sanitize_key_name("alt+Tab").is_ok());
        assert!(sanitize_key_name("; rm -rf /").is_err());
        assert!(sanitize_key_name("").is_err());
    }

    #[test]
    fn gui_guard_grant_consume() {
        let mut g = GuiGuard::default();
        assert!(!g.is_granted());
        assert!(!g.consume()); // 未授权不消费
        g.add_token();
        assert!(g.is_granted());
        assert!(g.consume()); // 消费 token
        assert!(!g.is_granted());
        g.grant();
        assert!(g.consume());
        g.revoke();
        assert!(!g.is_granted());
    }
}
