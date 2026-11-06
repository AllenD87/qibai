//! 屏幕「眼睛 + 手」执行层（v18.3）：全屏截图、xdotool 键鼠/滚动、窗口管理。
//!
//! 安全设计：
//! - 命令一律参数数组调用（`Command::new(...).args(...)`），**无 shell 拼接**，防命令注入；
//! - 键鼠操作必须经 `qibai_core::screen::GuiGuard` 分级授权（语音开权限 / 面板点击=临时授权一次）；
//! - 输入文本与键名过 `sanitize_input_text` / `sanitize_key_name`（控制字符/白名单外拒绝）；
//! - 屏幕内容只作「展示/理解」，操作只由用户显式触发（防 Prompt 注入隔离）。

use std::process::Command;

/// which 探测工具是否存在。
fn which(tool: &str) -> bool {
    Command::new("which")
        .arg(tool)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 当前可用的截图工具（按优先级），无则 None。
pub fn capture_tool_available() -> Option<String> {
    for t in qibai_core::screen::CAPTURE_TOOLS {
        if which(t) {
            return Some(t.to_string());
        }
    }
    None
}

/// 从 PNG 字节解析宽高（IHDR 大端）。
fn png_size(bytes: &[u8]) -> (u32, u32) {
    if bytes.len() >= 24 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return (w, h);
    }
    (0, 0)
}

/// 全屏截图 → (base64_png, width, height)。
/// 截图存临时目录后读回并清理，不留敏感文件。
pub fn capture_png_base64() -> Result<(String, u32, u32), String> {
    let tool = capture_tool_available().ok_or_else(|| {
        "未检测到截图工具，请先安装：sudo apt install imagemagick scrot xdotool wmctrl".to_string()
    })?;
    let out = std::env::temp_dir().join(format!(
        "qibai-screen-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    ));
    let out_str = out.to_str().ok_or("截图路径非法")?.to_string();
    let argv = qibai_core::screen::capture_argv(&tool, &out_str)
        .ok_or("截图工具参数构造失败")?;
    let st = Command::new(&argv[0])
        .args(&argv[1..])
        .output()
        .map_err(|e| format!("截图执行失败：{e}"))?;
    if !st.status.success() {
        return Err(format!(
            "截图失败（工具 {tool}）退出码 {:?}",
            st.status.code()
        ));
    }
    let bytes = std::fs::read(&out).map_err(|e| format!("读取截图失败：{e}"))?;
    let (w, h) = png_size(&bytes);
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    let _ = std::fs::remove_file(&out);
    Ok((b64, w, h))
}

/// xdotool 执行器（参数数组调用）。
fn xdo(args: &[String]) -> Result<(), String> {
    if !which("xdotool") {
        return Err("未安装 xdotool，请先安装：sudo apt install xdotool".to_string());
    }
    let st = Command::new("xdotool")
        .args(args)
        .output()
        .map_err(|e| format!("xdotool 执行失败：{e}"))?;
    if !st.status.success() {
        return Err(format!(
            "xdotool {:?} 失败，退出码 {:?}",
            args,
            st.status.code()
        ));
    }
    Ok(())
}

/// 鼠标点击（按钮 1=左键）。
pub fn click(x: u32, y: u32) -> Result<(), String> {
    xdo(&[
        "mousemove".into(),
        x.to_string(),
        y.to_string(),
        "click".into(),
        "1".into(),
    ])
}

/// 输入文本（单参数传递，防注入；延迟 30ms 模拟自然输入）。
pub fn type_text(text: &str) -> Result<(), String> {
    let safe = qibai_core::screen::sanitize_input_text(text)?;
    xdo(&["type".into(), "--delay".into(), "30".into(), safe])
}

/// 按键（键名白名单由 qibai_core 校验）。
pub fn key(keys: &str) -> Result<(), String> {
    let safe = qibai_core::screen::sanitize_key_name(keys)?;
    xdo(&["key".into(), safe])
}

/// 滚动：dy>0 向下滚，dy<0 向上滚（每格一次，clamp ±20 格防刷屏）。
pub fn scroll(dy: i32) -> Result<(), String> {
    let times = dy.clamp(-20, 20).unsigned_abs();
    if times == 0 {
        return Ok(());
    }
    let btn = if dy > 0 { "5" } else { "4" };
    let mut args: Vec<String> = Vec::with_capacity(4);
    for _ in 0..times.min(20) {
        args.push("click".into());
        args.push(btn.into());
    }
    xdo(&args)
}

/// 激活窗口（按标题子串；窗口标题经长度/控制字符校验，防注入）。
pub fn activate_window(title: &str) -> Result<(), String> {
    let t = title.trim();
    if t.is_empty() {
        return Err("窗口标题为空".to_string());
    }
    if t.chars().count() > 80 || t.chars().any(|c| c.is_control()) {
        return Err("窗口标题非法，已拒绝".to_string());
    }
    if !which("wmctrl") {
        return Err("未安装 wmctrl，请先安装：sudo apt install wmctrl".to_string());
    }
    let st = Command::new("wmctrl")
        .args(["-a", t])
        .output()
        .map_err(|e| format!("wmctrl 执行失败：{e}"))?;
    if !st.status.success() {
        return Err(format!("未找到匹配窗口：{t}"));
    }
    Ok(())
}
