// ！！！注意：本 crate 依赖 tauri 及其 sys 系 crate（webkit2gtk-4.1 / gtk3 /
// libsoup-3.0 / javascriptcoregtk-4.1）。在缺这些系统库的 Linux 环境下无法编译，
// 这是预期行为——根 workspace 不包含本 crate。构建方法见 docs/BUILD_NOTE.md。
//
// 双窗口架构（docs/WINDOW_ARCH.md）：
// - 所有窗口运行时创建（tauri.conf.json app.windows 为空数组）；
// - 桌宠窗 `pet-<persona_id>`：220×260，透明无边框置顶，每在场角色一窗；
// - 控制台窗 `console`：460×680，按需呼出，关闭即销毁；
// - qibai-core 是唯一状态源，本文件只做窗口协调 + 事件广播。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod listener;
mod ptt;
mod read_cache;
mod screen_gui;

use qibai_core::{Engine, EventSink, ReminderEvent, SendMessageOutput, Settings};
use read_cache::{ReadCache, force_refresh, spawn_refresher};
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::async_runtime::{Mutex, RwLock};
use cpal::traits::{DeviceTrait, HostTrait};
use tauri::{Emitter, Listener, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_autostart::MacosLauncher;
use tokio::time::{interval, sleep};

/// 把 Engine 事件桥接到 tauri 前端事件系统。
struct TauriSink {
    app: tauri::AppHandle,
}

impl EventSink for TauriSink {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = self.app.emit(event, payload);
    }
}

/// 共享引擎：命令侧与后台提醒 tick 侧共用。
struct AppState {
    engine: Arc<Mutex<Engine>>,
    pet_ready: Arc<AtomicBool>,
    listen: Arc<listener::ListenerShared>,
    /// v18.3 GUI 分级授权（键鼠操作开关 + 会话临时授权次数）。同步锁：不跨 await 持有。
    gui: Arc<std::sync::Mutex<qibai_core::screen::GuiGuard>>,
    /// v45.3 只读快照：长任务持 engine 锁期间，所有只读命令改从这里读，
    /// 不再排队在 write 后面。见 read_cache.rs。
    read_cache: Arc<RwLock<ReadCache>>,
    /// v45.21 用户叫停标志。**刻意放在 engine 锁之外**——`send_message` 会整条链
    /// 持着 engine 锁 await，若 stop 命令也去抢这把锁，就会被堵到任务跑完，
    /// 等于没有取消。停止只置位这个原子量，工具循环自己在检查点上收手。
    cancel: Arc<AtomicBool>,
}

/// X11 软件渲染且缺少可靠合成时，透明窗会保留旧帧表面。此时创建不透明兼容窗，
/// 前端同步关闭透明画布，杜绝“开关显示开启、实际不断叠影”。其他平台/硬件路径保持原生透明。
static PET_NATIVE_TRANSPARENCY: AtomicBool = AtomicBool::new(true);

/// v45.5：无合成器自动降级标记。置位后：
/// - pet_surface_rebuild / pet_invalidate_rect 直接跳过（这两条 v45 新增的表面操作
///   是崩溃最大嫌疑，在不透明窗上无意义且会刺激 X/GTK 路径）。
static AUTO_DEGRADED_OPAQUE: AtomicBool = AtomicBool::new(false);

/// v45.4 safe-mode 开关（启动时读一次 env，全程缓存）。
/// 三个独立、可组合：
///   QIBAI_SAFE_OPAQUE=1            → 所有窗强制不透明（绕开 WebKitGTK 透明 ARGB + 无合成器 SIGTRAP）
///   QIBAI_SAFE_NO_AUDIO=1          → 启动期不初始化 cpal/listener/whisper 自检
///   QIBAI_SAFE_NO_COMPOSITOR_PROBE=1 → 不跑 xprop/_NET_WM_CM_S0 探测
fn env_flag(name: &str) -> bool {
    std::env::var_os(name).map(|v| v == "1").unwrap_or(false)
}
fn safe_opaque() -> bool { env_flag("QIBAI_SAFE_OPAQUE") }
fn safe_no_audio() -> bool { env_flag("QIBAI_SAFE_NO_AUDIO") }
fn safe_no_compositor_probe() -> bool { env_flag("QIBAI_SAFE_NO_COMPOSITOR_PROBE") }

fn pet_native_transparency() -> bool {
    // v45.4：safe opaque 优先级最高——真机刚杀 compiz、无 WM 时透明窗必崩。
    if safe_opaque() {
        return false;
    }
    PET_NATIVE_TRANSPARENCY.load(Ordering::Relaxed)
}

/// v45 热切换：全程只有一个桌宠窗，label 固定为 `pet-stage`（仍以 `pet-` 开头，
/// 不破坏所有 `starts_with("pet-")` 的定位/形态/置顶逻辑）。切角色在窗内热替换
/// 立绘/音色/问候语，窗口几何全程不变——从结构上杜绝「两个角色窗短暂同屏」。
const STAGE_LABEL: &str = "pet-stage";

#[tauri::command]
fn pet_render_info() -> serde_json::Value {
    let native = pet_native_transparency();
    serde_json::json!({
        "transparent": native,
        "mode": if native { "native-transparent" } else { "x11-safe-opaque" }
    })
}

// ---------- 窗口协调 helpers ----------

/// 当前 label 前缀为 `pet-` 的桌宠窗数量（召唤错位移位用）。
fn pet_window_count(app: &tauri::AppHandle) -> usize {
    app.webview_windows()
        .iter()
        .filter(|(_, w)| w.label().starts_with("pet-"))
        .count()
}

/// 创建一个桌宠窗。第 n 个召唤窗按契约 §3 错开位置：x += 48*n, y += 36*n。
fn create_pet_window(
    app: &tauri::AppHandle,
    _persona_id: &str,
) -> tauri::Result<tauri::WebviewWindow> {
    let n = pet_window_count(app) as f64;
    let mut builder = WebviewWindowBuilder::new(
        app,
        STAGE_LABEL,
        WebviewUrl::App("pet.html".into()),
    )
    // v30：340×330——角色立绘完整放下 + 顶部留出「头顶气泡」区(18~58px) + 底部输入条。
    .inner_size(360.0, 380.0)
    .title("元气小小·桌宠")
    .transparent(pet_native_transparency())
    .decorations(false)
    .always_on_top(true)
    .resizable(false)
    .shadow(false)
    .skip_taskbar(true)
    // v44：隐藏创建——等页面首帧就绪（on_page_load Finished）再 show，
    // 彻底杜绝「旧窗尚未销毁、新 pet.html 已在同地亮帧」的新旧立绘同屏叠影。
    .visible(false);
    // 首窗默认贴屏幕右缘（贴边小窗）；多 persona 召唤才错位 48n/36n。
    if n == 0.0 {
        if let Ok(Some(mon)) = app.primary_monitor() {
            let wa = mon.work_area();
            let (mx, my) = (wa.position.x as f64, wa.position.y as f64);
            let (mw, mh) = (wa.size.width as f64, wa.size.height as f64);
            // v30：贴边宽度 220→340 修正（窗口加宽后旧值让右缘伸出屏外 120px）；
            // 高度 310→330 与 inner_size 同步。
            builder = builder.position(
                (mx + mw - 360.0 - 10.0).max(0.0),
                (my + mh - 380.0 - 60.0).max(0.0),
            );
        }
    } else {
        builder = builder.position(48.0 * n, 36.0 * n);
    }
    // 首帧 DOM 就绪后再显示（Finished = 页面加载完成），并立即重申置顶 + reflow 清缓冲。
    let w = builder
        .on_page_load(|win, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Finished {
                let _ = win.show();
                let _ = win.set_always_on_top(true);
                let _ = win.eval(PET_REFLOW_JS);
            }
        })
        .build()?;
    // 兜底：on_page_load 若因缓存/异常未触发，1.2s 后强制 show，防窗口永久隐藏。
    let w_fb = w.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        let _ = w_fb.show();
        let _ = w_fb.set_always_on_top(true);
    });
    Ok(w)
}

/// UI 形态切换执行器（voice_ui_cmd 的 PetMode 落点，listener.rs 调用）。
/// compact/restore = 贴右缘小窗 220×310（角色完整显示）+ 极简前端；
/// large = 居中大窗 420×600 + 完整前端界面；min = 最小化。
/// 对所有 pet-* 窗口统一生效；窗口缺失幂等返回 Ok。
pub fn apply_pet_mode(app: &tauri::AppHandle, mode: &str) -> Result<(), String> {
    let labels: Vec<String> = app
        .webview_windows()
        .keys()
        .filter(|l| l.starts_with("pet-"))
        .cloned()
        .collect();
    if labels.is_empty() {
        return Ok(());
    }
    let monitor = app
        .get_webview_window(&labels[0])
        .and_then(|w| w.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or_else(|| "no monitor".to_string())?;
    let wa = monitor.work_area();
    let (wa_x, wa_y) = (wa.position.x as f64, wa.position.y as f64);
    let (wa_w, wa_h) = (wa.size.width as f64, wa.size.height as f64);

    for label in &labels {
        let Some(w) = app.get_webview_window(label) else { continue };
        match mode {
            "large" => {
                let (ww, wh) = (420.0f64.min(wa_w * 0.5), 600.0f64.min(wa_h * 0.85));
                let _ = w.set_resizable(true);
                let _ = w.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(
                    ww as u32, wh as u32,
                )));
                let _ = w.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
                    (wa_x + (wa_w - ww) / 2.0).max(0.0) as i32,
                    (wa_y + (wa_h - wh) / 2.0).max(0.0) as i32,
                )));
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_always_on_top(true);
                let _ = w.set_focus();
                let _ = app.emit("qibai://ui-minimal", serde_json::json!({ "on": false }));
            }
            "min" => {
                let _ = w.minimize();
            }
            _ => {
                // compact / restore：贴右缘小窗，底部留任务栏空间。
                let (ww, wh) = (220.0, 310.0);
                let _ = w.set_resizable(false);
                let _ = w.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(
                    ww as u32, wh as u32,
                )));
                let _ = w.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
                    (wa_x + wa_w - ww - 10.0).max(0.0) as i32,
                    (wa_y + wa_h - wh - 60.0).max(0.0) as i32,
                )));
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_always_on_top(true);
                // Linux 兜底：强制 webview reflow，防 resize 后内容不重排/残留。
                let _ = w.eval(PET_REFLOW_JS);
                let _ = app.emit("qibai://ui-minimal", serde_json::json!({ "on": true }));
            }
        }
    }
    Ok(())
}

/// 控制台窗：存在则 show+focus，否则创建（520×760，正常标题栏，可缩放，非置顶）。
fn ensure_console(app: &tauri::AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("console") {
        let _ = w.show();
        let _ = w.set_focus();
    } else {
        // v13：控制台 460×680 → 520×760（tab 多、内容多，680 高在缩放屏易底部被裁）；
        // 创建后 center()，防无 position 时偏出屏幕被误判「显示不全」。
        let w = WebviewWindowBuilder::new(app, "console", WebviewUrl::App("console.html".into()))
            .inner_size(520.0, 760.0)
            .min_inner_size(480.0, 560.0) // v20：防缩放屏/拖动时窗口缩得太小导致内容显示不全
            .decorations(true)
            .resizable(true)
            .always_on_top(false)
            .skip_taskbar(false)
            .build()?;
        // v20：窗口尺寸超过显示器工作区时收缩到工作区内，杜绝「底部/右侧被裁、显示不全」。
        if let Ok(Some(m)) = app.primary_monitor() {
            let wa = m.work_area();
            let sz = w.outer_size().unwrap_or(tauri::PhysicalSize::new(520, 760));
            let (cw, ch) = (sz.width.min(wa.size.width), sz.height.min(wa.size.height));
            if cw != sz.width || ch != sz.height {
                let _ = w.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(cw, ch)));
            }
        }
        let _ = w.center();
    }
    Ok(())
}

/// v20收尾：关闭控制台窗（withGlobalTauri=false 后 console.js 的 window.close() 在
/// Tauri webview 无效，改走本命令）。找不到 console 窗幂等返回 Ok。
/// 契约：`invoke('close_console')`。
#[tauri::command]
fn close_console(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("console") {
        w.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// ============================================================
/// v21.11 架构根治：右键菜单改为【独立不透明菜单窗】。
/// 背景：菜单作为透明小窗内浮层，在无合成器 X11（compositor: MISSING）下
/// display:none 的旧像素物理清不掉 →「菜单藏在小女孩背后不消失」。
/// 根治：菜单放独立不透明窗（menu.html）——不透明窗无合成器也正常合成，
/// 关闭即销毁（X 窗口销毁 = 物理消失），与立绘窗完全分离，杜绝遮挡/残留/穿透。
/// ============================================================
#[tauri::command]
fn ctx_menu_open(app: tauri::AppHandle, x: i32, y: i32) -> Result<(), String> {
    const MW: i32 = 180;
    const MH: i32 = 290; // v21.11.7：5 项+2 分组标签实测高约 238px，240 窗口放不下，底部「退出」被截断
    // 菜单窗应出现在「小窗内鼠标位置」的屏幕坐标 = 小窗外置位置 + 鼠标偏移
    let mut pos = tauri::LogicalPosition::new(0.0_f64, 0.0_f64);
    for (_, p) in app.webview_windows() {
        if p.label().starts_with("pet-") {
            if let Ok(pp) = p.outer_position() {
                pos = tauri::LogicalPosition::new(pp.x as f64 + x as f64, pp.y as f64 + y as f64);
            }
            break;
        }
    }
    // 复用已有菜单窗（直接移动+置顶），否则创建（透明、无边框、无阴影、置顶、不占任务栏）
    if let Some(w) = app.get_webview_window("ctx-menu") {
        let _ = w.set_position(pos);
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let mut menu_builder = WebviewWindowBuilder::new(&app, "ctx-menu", WebviewUrl::App("menu.html?v=6".into()))
        .inner_size(MW as f64, MH as f64)
        .decorations(false)
        // v40：与主窗同一套透明参数。缺 transparent(true) 时窗口底压着一层不透明白底，
        // menu.html 的 24px 圆角卡片就落在矩形上 —— 真机即「右键还有矩形框、不是纯圆角」。
        // v45.4：safe opaque 时跳过 transparent(true)，绕开无合成器 ARGB 崩溃。
        .shadow(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .visible(false);
    if !safe_opaque() {
        menu_builder = menu_builder.transparent(true);
    }
    // v34：先隐藏构建→定位→再显示，消除「默认位置闪现矩形再移动」。
    let w = menu_builder
        .build()
        .map_err(|e| e.to_string())?;
    // 越界保护：夹取到工作区内，防菜单窗跑到屏幕外「显示不全」
    if let Ok(Some(m)) = app.primary_monitor() {
        let wa = m.work_area();
        let (waw, wah) = (wa.size.width as i32, wa.size.height as i32);
        let px = (pos.x as i32).clamp(wa.position.x, wa.position.x + waw - MW);
        let py = (pos.y as i32).clamp(wa.position.y, wa.position.y + wah - MH);
        let _ = w.set_position(tauri::LogicalPosition::new(px as f64, py as f64));
    } else {
        let _ = w.set_position(pos);
    }
    let _ = w.show();
    let _ = w.set_focus();
    Ok(())
}

/// 菜单窗点击某项：销毁菜单窗（物理消失）+ 执行动作。
/// v21.11.3：console/exit 由后端**直接执行**（不再走 emit→前端→invoke 回后端，
/// 少一跳少一个断点——之前「菜单关了但控制台不弹」就是这条链路在真机断了）；
/// lock/guard/minimal 仍 emit 给立绘窗（它们依赖前端 state/DOM），目标改为
/// **全部非菜单窗**（消除 label 前缀匹配不到的隐患）；全程打日志便于定位。
#[tauri::command]
fn ctx_menu_pick(app: tauri::AppHandle, action: String) -> Result<(), String> {
    let labels: Vec<String> = app.webview_windows().keys().cloned().collect();
    println!("[qibai] ctx_menu_pick action={} windows={:?}", action, labels);
    if let Some(w) = app.get_webview_window("ctx-menu") {
        let _ = w.close();
    }
    match action.as_str() {
        "console" => {
            // 后端直接弹控制台窗（ensure_console 幂等：存在则显示/聚焦）
            let _ = ensure_console(&app);
            println!("[qibai] ctx_menu_pick console ensure_console done");
        }
        "exit" => {
            println!("[qibai] ctx_menu_pick exit");
            app.exit(0);
        }
        _ => {
            println!("[qibai] ctx-action emit: {}", action); // v35 诊断：确认后端收到并广播 transbg
            for (_, p) in app.webview_windows() {
                if p.label() != "ctx-menu" {
                    let r = p.emit("qibai://ctx-action", serde_json::json!({ "action": action }));
                    if let Err(e) = r {
                        println!("[qibai] ctx_menu_pick emit fail to {}: {:?}", p.label(), e);
                    }
                }
            }
        }
    }
    Ok(())
}

/// 菜单窗失焦/ESC/点空白：销毁菜单窗（幂等）。
#[tauri::command]
fn ctx_menu_close(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("ctx-menu") {
        w.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 独立面板窗（v18 UI 面板：大内容用独立窗口）。存在则复用并调整尺寸，否则创建。
/// 面板窗加载 `panel.html`（CSP 严格、只渲染白名单 HTML，见 qibai-core::panel）。
fn ensure_panel_window(
    app: &tauri::AppHandle,
    w: f64,
    h: f64,
) -> tauri::Result<tauri::WebviewWindow> {
    if let Some(win) = app.get_webview_window("panel") {
        let _ = win.set_size(tauri::LogicalSize::new(w.max(320.0), h.max(240.0)));
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(win);
    }
    WebviewWindowBuilder::new(app, "panel", WebviewUrl::App("panel.html".into()))
        .inner_size(w.max(320.0), h.max(240.0))
        .title("元气小小 · 面板")
        .decorations(true)
        .resizable(true)
        .always_on_top(false)
        .skip_taskbar(false)
        .build()
}

/// v18 UI 面板总入口：mode=bubble → 桌宠窗气泡卡片；mode=window（默认）→ 独立面板窗。
/// payload: { mode?, title, html, w?, h? }
/// html 一律经 qibai_core::panel::sanitize_panel_html 白名单消毒后再渲染。
#[tauri::command]
async fn ui_panel_show(app: tauri::AppHandle, payload: serde_json::Value) -> Result<(), String> {
    let mode = payload["mode"].as_str().unwrap_or("window").to_string();
    let title = payload["title"].as_str().unwrap_or("元气小小").to_string();
    let raw = payload["html"].as_str().unwrap_or("");
    let html = qibai_core::panel::sanitize_panel_html(raw);
    if mode == "bubble" {
        let _ = app.emit(
            "qibai://ui-panel",
            serde_json::json!({ "title": title, "html": html }),
        );
        return Ok(());
    }
    let w = payload["w"].as_f64().unwrap_or(660.0);
    let h = payload["h"].as_f64().unwrap_or(520.0);
    let _win = ensure_panel_window(&app, w, h).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "qibai://panel-content",
        serde_json::json!({ "title": title, "html": html }),
    );
    Ok(())
}

/// 关闭独立面板窗（panel.html 右上角/前端按钮调用；语音「关掉面板」也可直达）。
#[tauri::command]
async fn ui_panel_close(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.close();
    }
    Ok(())
}

// ---------- v18.3 屏幕「眼睛 + 手」命令 ----------

/// 全屏截图 → 面板窗展示可点击画面（低危，直接可用）。
/// 图片经 base64 data URI 传给前端（CSP img-src data: 已允许），不留敏感文件。
#[tauri::command]
async fn screen_capture(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let (b64, w, h) = screen_gui::capture_png_base64()?;
    let _win = ensure_panel_window(&app, 920.0, 680.0).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "qibai://screen-shot",
        serde_json::json!({ "image": b64, "w": w, "h": h }),
    );
    Ok(serde_json::json!({ "ok": true, "w": w, "h": h }))
}

/// GUI 授权状态查询（前端状态条显示）。
#[tauri::command]
async fn screen_gui_state(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let g = state.gui.lock().map_err(|_| "GUI 状态锁冲突".to_string())?;
    Ok(serde_json::json!({ "enabled": g.enabled, "granted": g.is_granted() }))
}

/// 语音「打开操作权限 / 关闭操作权限」→ 开关 GUI 键鼠授权。
#[tauri::command]
async fn screen_gui_set(state: tauri::State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let mut g = state.gui.lock().map_err(|_| "GUI 状态锁冲突".to_string())?;
    if enabled {
        g.grant();
    } else {
        g.revoke();
    }
    Ok(())
}

/// 面板点击 = 用户显式授权一次（临时 token，用后即焚）。
#[tauri::command]
async fn screen_gui_ack(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.gui.lock().map_err(|_| "GUI 状态锁冲突".to_string())?.add_token();
    Ok(())
}

/// 鼠标点击（面板主动点击已临时授权；语音坐标点击需已开启操作权限）。
#[tauri::command]
async fn screen_click(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    x: u32,
    y: u32,
) -> Result<(), String> {
    let mut g = state.gui.lock().map_err(|_| "GUI 状态锁冲突".to_string())?;
    if !g.consume() {
        return Err("操作权限未开启：请先语音说「打开操作权限」，或直接在截图面板上点击目标位置".to_string());
    }
    drop(g);
    screen_gui::click(x, y).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "qibai://pet-say",
        serde_json::json!({ "persona_id": "default", "text": format!("点了一下 ({x}, {y})。"), "ms": 2000 }),
    );
    Ok(())
}

/// 输入文本（需授权；文本过控制字符/长度校验）。
#[tauri::command]
async fn screen_type(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    text: String,
) -> Result<(), String> {
    let mut g = state.gui.lock().map_err(|_| "GUI 状态锁冲突".to_string())?;
    if !g.consume() {
        return Err("操作权限未开启：请先语音说「打开操作权限」".to_string());
    }
    drop(g);
    screen_gui::type_text(&text).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "qibai://pet-say",
        serde_json::json!({ "persona_id": "default", "text": "输入完成。", "ms": 2000 }),
    );
    Ok(())
}

/// 按键（键名白名单校验）。
#[tauri::command]
async fn screen_key(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    keys: String,
) -> Result<(), String> {
    let mut g = state.gui.lock().map_err(|_| "GUI 状态锁冲突".to_string())?;
    if !g.consume() {
        return Err("操作权限未开启：请先语音说「打开操作权限」".to_string());
    }
    drop(g);
    screen_gui::key(&keys).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "qibai://pet-say",
        serde_json::json!({ "persona_id": "default", "text": "按键完成。", "ms": 2000 }),
    );
    Ok(())
}

/// 滚动（dy>0 向下）。
#[tauri::command]
async fn screen_scroll(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    dy: i32,
) -> Result<(), String> {
    let mut g = state.gui.lock().map_err(|_| "GUI 状态锁冲突".to_string())?;
    if !g.consume() {
        return Err("操作权限未开启：请先语音说「打开操作权限」".to_string());
    }
    drop(g);
    screen_gui::scroll(dy).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "qibai://pet-say",
        serde_json::json!({ "persona_id": "default", "text": if dy > 0 { "向下滚了滚。" } else { "向上滚了滚。" }, "ms": 2000 }),
    );
    Ok(())
}

/// 激活窗口（按标题）。
#[tauri::command]
async fn screen_activate_window(state: tauri::State<'_, AppState>, title: String) -> Result<(), String> {
    let mut g = state.gui.lock().map_err(|_| "GUI 状态锁冲突".to_string())?;
    if !g.consume() {
        return Err("操作权限未开启：请先语音说「打开操作权限」".to_string());
    }
    drop(g);
    screen_gui::activate_window(&title).map_err(|e| e.to_string())
}

/// 内置面板：最近聊天记录 → 独立面板窗表格（供语音/前端直接调用，不走 LLM）。
#[tauri::command]
async fn chat_log_panel(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    limit: Option<usize>,
) -> Result<(), String> {
    let msgs = state.engine.lock().await.get_conversation().to_vec();
    let html = qibai_core::panel::chat_log_table(&msgs, limit.unwrap_or(30));
    let _ = ensure_panel_window(&app, 720.0, 560.0).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "qibai://panel-content",
        serde_json::json!({ "title": "最近聊天记录", "html": html }),
    );
    Ok(())
}

/// 内置面板：已装配技能列表 → 独立面板窗表格。
#[tauri::command]
async fn skills_panel(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let skills = state.read_cache.read().await.skill_list.clone();
    let html = qibai_core::panel::skill_table(&skills);
    let _ = ensure_panel_window(&app, 760.0, 560.0).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "qibai://panel-content",
        serde_json::json!({ "title": "技能库", "html": html }),
    );
    Ok(())
}

/// 内置面板：已装配专家列表 → 独立面板窗表格。
#[tauri::command]
async fn experts_panel(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let experts = state.read_cache.read().await.expert_list.clone();
    let html = qibai_core::panel::expert_table(&experts);
    let _ = ensure_panel_window(&app, 720.0, 540.0).map_err(|e| e.to_string())?;
    let _ = app.emit(
        "qibai://panel-content",
        serde_json::json!({ "title": "专家库", "html": html }),
    );
    Ok(())
}

/// 广播 desk-changed：payload 为引擎 desk_state（`{present, locked}`）。
async fn broadcast_desk_changed(app: &tauri::AppHandle, engine: &Arc<Mutex<Engine>>) {
    let state = engine.lock().await.desk_state();
    let _ = app.emit("qibai://desk-changed", state);
}

/// 把一组台词逐条广播到对应桌宠窗（前端按 persona_id 过滤，契约 §5）。
fn broadcast_pet_say_lines(app: &tauri::AppHandle, lines: &[serde_json::Value]) {
    for line in lines {
        let speaker = line["speaker"].as_str().unwrap_or("");
        let text = line["text"].as_str().unwrap_or("");
        let _ = app.emit(
            "qibai://pet-say",
            serde_json::json!({ "persona_id": speaker, "text": text }),
        );
    }
}

// ---------- 既有命令（契约 §2.1，保持不动） ----------

#[tauri::command]
async fn get_settings(state: tauri::State<'_, AppState>) -> Result<Settings, String> {
    // v45.3：改走只读快照，不再排队在长任务锁后面。
    if let Some(s) = &state.read_cache.read().await.settings {
        return Ok(s.clone());
    }
    Ok(state.engine.lock().await.get_settings())
}

/// v18.8：枚举系统音频输入/输出设备（设置页"麦克风选择"用）。
/// 输入设备带打分：mic/麦克/内置优先，hdmi/display/nvidia 降权（避免显卡输出被当麦克风）。
#[tauri::command]
async fn list_audio_devices() -> Result<serde_json::Value, String> {
    let host = cpal::default_host();
    let mut inputs: Vec<serde_json::Value> = Vec::new();
    if let Ok(devs) = host.input_devices() {
        for d in devs {
            if let Ok(n) = d.name() {
                let lower = n.to_lowercase();
                let mut sc = 0;
                if lower.contains("mic") || lower.contains("麦克") {
                    sc += 3;
                }
                if lower.contains("pulse") || lower == "default" || lower.contains("default") {
                    sc += 3;
                }
                if lower.contains("headset") || lower.contains("headphone") || lower.contains("耳机")
                    || lower.contains("耳麦") || lower.contains("handsfree") || lower.contains("hands-free")
                {
                    sc += 4;
                }
                if lower.contains("usb") {
                    sc += 2;
                }
                if lower.contains("analog") || lower.contains("内置") || lower.contains("input")
                    || lower.contains("microphone") || lower.contains("front")
                {
                    sc += 2;
                }
                if lower.contains("hw:") || lower.contains("plughw:") {
                    sc -= 20;
                }
                if lower.contains("monitor") || lower.contains("loopback") || lower.contains("echo")
                    || lower.contains("feedback")
                {
                    sc -= 5;
                }
                if lower.contains("hdmi") || lower.contains("display") || lower.contains("nvidia")
                    || lower.contains("dp ")
                {
                    sc -= 2;
                }
                inputs.push(serde_json::json!({ "name": n, "score": sc }));
            }
        }
    }
    let mut outputs: Vec<serde_json::Value> = Vec::new();
    if let Ok(devs) = host.output_devices() {
        let default_name = host.default_output_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();
        for d in devs {
            if let Ok(n) = d.name() {
                outputs.push(serde_json::json!({
                    "name": n,
                    "is_default": n == default_name,
                }));
            }
        }
    }
    inputs.sort_by(|a, b| {
        b["score"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&a["score"].as_i64().unwrap_or(0))
    });
    Ok(serde_json::json!({ "inputs": inputs, "outputs": outputs }))
}

#[tauri::command]
async fn save_settings(state: tauri::State<'_, AppState>, settings: Settings) -> Result<(), String> {
    let r = state
        .engine
        .lock().await
        .save_settings(settings)
        .map_err(|e| e.to_string());
    // v45.3：写后立即刷快照，避免前端只读接口还读到旧设置。
    force_refresh(&state.engine, &state.read_cache).await;
    r
}

/// v45.10：JSON Merge Patch（RFC 7386）——对象递归合并、null 删除键、未出现字段保留旧值。
/// 前端 collect() 只传"UI 上能编辑的字段"，不再拼完整 Settings。
#[tauri::command]
async fn save_settings_partial(
    state: tauri::State<'_, AppState>,
    patch: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let out = state
        .engine
        .lock().await
        .save_settings_partial(patch)
        .map_err(|e| e.to_string())?;
    force_refresh(&state.engine, &state.read_cache).await;
    serde_json::to_value(&out).map_err(|e| e.to_string())
}

/// v45.21：用户主动叫停当前这一轮。
/// 只置位锁外的原子量——**不去 lock engine**，否则会被正在跑的 send_message
/// 堵在队列里直到任务结束，命令等于无效。软停语义：当前工具跑完就刹车。
#[tauri::command]
fn stop_message(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    state.cancel.store(true, Ordering::SeqCst);
    eprintln!("[qibai] stop_message: 已置位取消标志（当前工具跑完后生效）");
    Ok(true)
}

/// v45.21：前端判断当前是否处于「已请求停止、但引擎还没收手」的过渡态，
/// 用来把按钮文案显示成「正在停…」而不是让用户以为没反应。
#[tauri::command]
fn stop_state(state: tauri::State<'_, AppState>) -> bool {
    state.cancel.load(Ordering::SeqCst)
}

#[tauri::command]
async fn send_message(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    text: String,
) -> Result<SendMessageOutput, String> {
    // v45.8：双保险——空文本直接拦（防止设备丢失后旧帧/空转写被当新输入发出去）
    if text.trim().is_empty() {
        eprintln!("[qibai] send_blocked: empty text");
        return Err("空消息，已忽略".to_string());
    }
    let out = {
        // v45.21：开新一轮先把取消标志清零（上一轮可能留下了 true）；
        // clone 一份 Arc 传进引擎，停止命令改的是同一个原子量。
        state.cancel.store(false, Ordering::SeqCst);
        let cancel = state.cancel.clone();
        let mut engine = state.engine.lock().await;
        engine.send_message_cancellable(&text, cancel.as_ref()).await
    };
    let result = match out {
        Ok(o) => {
            // v18.8-fix31：Rust 端朗读兜底——流式/非流式统一由 Rust 播放完整回复全文，
            // 彻底不依赖前端 JS 事件触发链（真机实证前端 tts-chunk 触发缺口导致无声）。
            // 前端只负责渲染气泡（engine 流式 emit 带 rust_played=true，前端不重复朗读）；
            // 非 secret（明文回复）才朗读，secret 回复只显示。
            if o.secret != Some(true) && !o.reply.trim().is_empty() {
                let req = {
                    let eng = state.engine.lock().await;
                    let pid = eng.personas.active_persona().to_string();
                    eng.tts_decide(tts::TtsKind::Chat, &o.reply, Some(&pid))
                };
                if let Some(r) = req {
                    // 借鉴 Qwen-Audio-Agent「完成≠交付」：结果就绪≠现在抢播。
                    // 用户正在说话或口播刚结束（尾音窗）→ 本轮跳过口播，
                    // 气泡已由 engine 流式呈现（内容不丢），等窗口安全再自然说话。
                    let win = qibai_core::tts::delivery_window_now();
                    if win.is_safe() {
                        perform_speech(&app, &state.engine, r).await;
                    } else {
                        eprintln!(
                            "[qibai] send_message 投递窗口不安全(user_speaking={}, playback={})→ 本轮仅气泡不口播",
                            win.user_speaking, win.playback_active
                        );
                    }
                } else {
                    eprintln!("[qibai] send_message Rust 朗读兜底: TTS 未启用(mute)——回复只显示不朗读");
                }
            }
            // v23：LLM 智能调度桌宠功能——LLM 回复里带 ui_cmd 时执行（白名单校验防幻觉/防越权）。
            // 执行器不写会话不播报（LLM 回复已写已播），这里只执行功能并留日志。
            if let Some(uc) = &o.ui_cmd {
                if let (Some(name), Some(params)) = (
                    uc.get("cmd").and_then(|v| v.as_str()),
                    uc.get("params"),
                ) {
                    if let Some(m) = qibai_core::voice_ui_cmd::ui_cmd_from_llm(name, params) {
                        eprintln!("[qibai] LLM ui_cmd 调度: {name} params={params}");
                        let m =
                            qibai_core::voice_ui_cmd::UiCmdMatch { cmd: m, reply: String::new(), args: params.clone() };
                        let reply = listener::run_voice_ui_cmd(&app, &state.engine, m).await;
                        eprintln!("[qibai] LLM ui_cmd 执行: {reply}");
                    } else {
                        eprintln!("[qibai] LLM ui_cmd 白名单拒绝: {name}");
                    }
                }
            }
            Ok(o)
        }
        Err(e) => {
            // v45.7：把真因打到 app.log，不再被前端 catch 吞成"没听清"。
            eprintln!("[qibai] send_message err: {e}");
            Err(e.to_string())
        }
    };
    // v45.3：send_message 整轮跑完（无论成功失败）立即刷快照，
    // 让新会话/新技能状态在 UI 立刻可见，不必等下一个 3s tick。
    force_refresh(&state.engine, &state.read_cache).await;
    match result {
        Ok(o) => Ok(o),
        Err(e) => Err(e),
    }
}

/// v30：打开本地工作区目录。
/// 口径直接取 qibai-core 的唯一根（`QIBAI_WORKSPACE` 覆盖，否则
/// `$HOME/Documents/qibai`）——这里以前自己维护了一份 `~/.qibai_workspace`，
/// 与真正写入文件的目录不一致，用户点「打开工作区」看到的是旧空目录、
/// 找不到刚生成的视频。
#[tauri::command]
fn code_open_workspace() -> Result<String, String> {
    let root = qibai_core::engine::qibai_workspace_root_public();
    let root = root.to_string_lossy().to_string();
    let _ = std::fs::create_dir_all(&root);
    let opened = open_in_file_manager(&root);
    if opened {
        Ok(format!("已打开本地工作区：{root}"))
    } else {
        Ok(format!(
            "工作区路径：{root}（未能自动打开文件管理器，请手动打开该目录）"
        ))
    }
}

/// 按平台唤起文件管理器。
fn open_in_file_manager(path: &str) -> bool {
    #[cfg(target_os = "windows")]
    let (prog, arg): (&str, Option<&str>) = ("explorer", Some(path));
    #[cfg(target_os = "macos")]
    let (prog, arg): (&str, Option<&str>) = ("open", Some(path));
    #[cfg(all(unix, not(target_os = "macos")))]
    let (prog, arg): (&str, Option<&str>) = ("xdg-open", Some(path));
    let mut cmd = std::process::Command::new(prog);
    if let Some(a) = arg {
        cmd.arg(a);
    }
    cmd.spawn().is_ok()
}

/// v31：打开本地知识库目录（与 qibai-core kb_search 同口径：
/// 默认 ~/.qibai_kb，环境变量 QIBAI_KB 可覆盖；放入 .txt/.md/.json 文档，
/// 问答时自动检索并整合进回答）。
#[tauri::command]
fn code_open_kb() -> Result<String, String> {
    let root = if let Ok(k) = std::env::var("QIBAI_KB") {
        let k = k.trim().to_string();
        if k.is_empty() { default_kb_root() } else { k }
    } else {
        default_kb_root()
    };
    let _ = std::fs::create_dir_all(&root);
    let opened = std::process::Command::new("xdg-open").arg(&root).spawn().is_ok();
    if opened {
        Ok(format!("已打开本地知识库：{root}"))
    } else {
        Ok(format!("知识库路径：{root}（未找到 xdg-open，请手动打开该目录）"))
    }
}

fn default_kb_root() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!("{home}/.qibai_kb")
}

/// v18.8-fix28：一键清空聊天记录与角色记忆（测试从零开始）。
#[tauri::command]
async fn clear_chat_history(state: tauri::State<'_, AppState>) -> Result<usize, String> {
    let mut eng = state.engine.lock().await;
    eng.clear_chat_history().map_err(|e| e.to_string())
}

/// v45.7：分范围清空聊天记录。
/// scope="chat" → 只清 conversation（保留角色记忆/亲密度）；
/// scope="all"  → 连角色记忆一起清（不可恢复）。
/// 返回 {cleared, backup, kept_memory}。
#[tauri::command]
async fn clear_conversation_scoped(
    state: tauri::State<'_, AppState>,
    scope: String,
) -> Result<serde_json::Value, String> {
    let include_memory = match scope.as_str() {
        "chat" => false,
        "all" => true,
        other => return Err(format!("invalid scope: {other:?} (expected \"chat\" or \"all\")")),
    };
    let mut eng = state.engine.lock().await;
    let (cleared, backup) = eng
        .clear_conversation(include_memory)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "cleared": cleared,
        "backup": backup.map(|p| p.display().to_string()),
        "kept_memory": !include_memory,
    }))
}

#[tauri::command]
async fn get_conversation(state: tauri::State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    // v45.3：改走只读快照，长任务跑时不再卡 130s。
    let list = state.read_cache.read().await.conversation.clone();
    eprintln!("[qibai] get_conversation(snapshot): {} 条", list.len());
    Ok(list)
}

#[tauri::command]
async fn get_skills_status(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    // v45.3：快照优先，未就绪再退回实时读。
    if let Some(v) = &state.read_cache.read().await.skills_status {
        return Ok(v.clone());
    }
    Ok(state.engine.lock().await.get_skills_status())
}

#[tauri::command]
async fn set_skill_enabled(
    state: tauri::State<'_, AppState>,
    skill: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .engine
        .lock().await
        .set_skill_enabled(&skill, enabled)
        .map_err(|e| e.to_string())
}

// ---- v45.8：技能安装器 IPC（core 侧 engine 提供实现） ----

/// v45.8：stage 技能安装包（url/file/dir 三种来源），返回摘要供前端确认。
#[tauri::command]
async fn skill_install_stage(
    state: tauri::State<'_, AppState>,
    kind: String,
    value: String,
) -> Result<serde_json::Value, String> {
    let source = serde_json::json!({ "kind": kind.clone(), "value": value.clone() });
    // v45.8：url 分支先下载（不持锁、不跨 await 占着 engine），拿到本地路径后再 stage。
    let staged_source = match kind.as_str() {
        "url" => {
            let tmp = std::env::temp_dir().join("qibai_skill_dl");
            std::fs::create_dir_all(&tmp).map_err(|e| format!("创建临时目录失败：{e}"))?;
            let path = qibai_core::skill_install::download_to_temp(&value, &tmp).await?;
            serde_json::json!({"kind": "file", "value": path.display().to_string()})
        }
        "file" | "dir" => source,
        other => return Err(format!("未知来源类型：{other}")),
    };
    state
        .engine
        .lock()
        .await
        .skill_install_stage(staged_source)
        .map_err(|e| e.to_string())
}

/// v45.8：commit 已 staged 的技能（overwrite=true 覆盖冲突）。
#[tauri::command(rename_all = "snake_case")]
async fn skill_install_commit(
    state: tauri::State<'_, AppState>,
    stage_id: String,
    overwrite: Option<bool>,
) -> Result<(), String> {
    let ov = overwrite.unwrap_or(false);
    state
        .engine
        .lock().await
        .skill_install_commit(&stage_id, ov)
        .map_err(|e| e.to_string())
}

/// v45.8：取消 staged 的安装，清理临时文件。
#[tauri::command(rename_all = "snake_case")]
async fn skill_install_cancel(
    state: tauri::State<'_, AppState>,
    stage_id: String,
) -> Result<(), String> {
    state
        .engine
        .lock().await
        .skill_install_cancel(&stage_id)
        .map_err(|e| e.to_string())
}

/// v45.8：卸载已安装的用户技能。
#[tauri::command]
async fn skill_uninstall(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    state
        .engine
        .lock().await
        .skill_uninstall(&name)
        .map_err(|e| e.to_string())
}

/// v45.8：读取技能的 SKILL.md 正文（只读弹层用）。
#[tauri::command]
async fn skill_read_doc(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<String, String> {
    state
        .engine
        .lock().await
        .skill_read_doc(&name)
        .map_err(|e| e.to_string())
}

/// v45.8：查询技能声明的权限列表。
#[tauri::command]
async fn skill_permissions(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<Vec<String>, String> {
    state
        .engine
        .lock().await
        .skill_permissions(&name)
        .map_err(|e| e.to_string())
}

// ---- v45.8：扬声器可选 IPC ----

/// 设置当前输出设备（"default" 或设备名）。
#[tauri::command]
async fn set_audio_output(device: String) -> Result<(), String> {
    qibai_core::tts::set_audio_output(&device);
    Ok(())
}

/// 读取当前输出设备名（"default" 或具体设备名）。
#[tauri::command]
async fn current_audio_output() -> Result<String, String> {
    Ok(qibai_core::tts::current_audio_output())
}

/// v45.8：设置搜索引擎（auto/baidu/360/sogou）。
#[tauri::command]
async fn set_search_engine(engine: String) -> Result<(), String> {
    qibai_core::web_search::set_search_engine(&engine);
    Ok(())
}

/// v45.8：读取当前搜索引擎偏好。
#[tauri::command]
async fn current_search_engine() -> Result<String, String> {
    Ok(qibai_core::web_search::current_search_engine())
}

// ---- v45.9：任务（task）IPC ----

/// v45.13：把 Task 序列化成的 JSON 补成前端契约形态。
///
/// 真实数据全部取自 task 内部状态（plan.steps 的 status / depends_on），前端不自己猜：
/// - 顶层补 `steps`（Rust 原本在 `plan.steps`，前端读平铺数组）；
/// - 每步补 `state` 别名（= status），并保留 `error` 原文供悬停；
/// - 补 `parallel`：`{max, groups, running, done, failed, progress:{done,total}}`。
///   `groups` 优先用 task 自带的 `parallel.groups`（调度线落盘后的真实拓扑）；
///   没有时按 `depends_on` 做拓扑分层兜底（无依赖的步骤同层 = 可并行候选）。
fn enrich_task_json(mut v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    let max = Settings::load().task_parallel_max;

    // 取 steps：优先 plan.steps，其次已有顶层 steps。
    let steps = v
        .get("plan")
        .and_then(|p| p.get("steps"))
        .cloned()
        .or_else(|| v.get("steps").cloned())
        .unwrap_or(Value::Array(vec![]));
    let steps_arr = if let Value::Array(a) = steps { a } else { Vec::new() };

    // 步骤状态分桶（真实取自 step.status）。
    let mut running: Vec<String> = Vec::new();
    let mut done: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut step_ids: Vec<String> = Vec::new();
    let mut steps_out: Vec<Value> = Vec::with_capacity(steps_arr.len());
    for mut s in steps_arr {
        let status = s
            .get("status")
            .and_then(|x| x.as_str())
            .unwrap_or("pending")
            .to_string();
        // 前端状态别名
        if s.get("state").is_none() {
            s["state"] = Value::String(status.clone());
        }
        let id = s
            .get("id")
            .and_then(|x| x.as_str())
            .map(|x| x.to_string())
            .unwrap_or_default();
        if !id.is_empty() {
            step_ids.push(id.clone());
            match status.as_str() {
                "running" | "waiting_confirm" => running.push(id.clone()),
                "done" => done.push(id.clone()),
                "failed" => failed.push(id.clone()),
                "skipped" => skipped.push(id.clone()),
                _ => {}
            }
        }
        steps_out.push(s);
    }

    // groups：优先真实拓扑；否则按 depends_on 拓扑分层兜底。
    let groups: Vec<Vec<String>> = v
        .get("parallel")
        .and_then(|p| p.get("groups"))
        .and_then(|g| g.as_array())
        .filter(|g| !g.is_empty())
        .map(|g| {
            g.iter()
                .filter_map(|grp| grp.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>()))
                .collect()
        })
        .unwrap_or_else(|| {
            // 兜底：按 depends_on 分层。无 depends_on 的步骤全部落在第 0 层（= 并行候选）。
            let mut deps: std::collections::HashMap<String, Vec<String>> = Default::default();
            for s in &steps_out {
                let id = s.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let d = s
                    .get("depends_on")
                    .and_then(|x| x.as_array())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>())
                    .unwrap_or_default();
                deps.insert(id, d);
            }
            let mut layers: Vec<Vec<String>> = Vec::new();
            let mut resolved: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut remaining: Vec<String> = step_ids.clone();
            while !remaining.is_empty() {
                let mut layer: Vec<String> = Vec::new();
                let mut still_left: Vec<String> = Vec::new();
                for id in remaining {
                    let deps_ok = deps
                        .get(&id)
                        .map(|d| d.iter().all(|d| resolved.contains(d)))
                        .unwrap_or(true);
                    if deps_ok {
                        layer.push(id.clone());
                        resolved.insert(id);
                    } else {
                        still_left.push(id);
                    }
                }
                if layer.is_empty() {
                    // 环 / 依赖缺失：把剩余的各自成组，避免死循环。
                    for id in still_left.drain(..) {
                        layers.push(vec![id]);
                    }
                    break;
                }
                layers.push(layer);
                remaining = still_left;
            }
            if layers.is_empty() { vec![vec![]] } else { layers }
        });

    let total = steps_out.len();
    let done_count = done.len() + skipped.len();
    v["steps"] = Value::Array(steps_out);
    // 状态别名：前端读 t.state。
    if v.get("state").is_none() {
        if let Some(st) = v.get("status").cloned() {
            v["state"] = st;
        }
    }
    v["parallel"] = serde_json::json!({
        "max": max,
        "groups": groups,
        "running": running,
        "done": done,
        "failed": failed,
        "skipped": skipped,
        "progress": { "done": done_count, "total": total },
    });
    v
}

/// v45.13：设置任务内并行分支上限（1..=8，默认 3）。
/// 走 save_settings_partial 落盘 + clamp_in_place（0/越界回 3 并留日志），立即生效；返回夹取后的新值。
#[tauri::command]
async fn task_set_parallel_max(
    state: tauri::State<'_, AppState>,
    n: u32,
) -> Result<serde_json::Value, String> {
    state
        .engine
        .lock().await
        .save_settings_partial(serde_json::json!({ "task_parallel_max": n }))
        .map_err(|e| e.to_string())?;
    force_refresh(&state.engine, &state.read_cache).await;
    let s = state.engine.lock().await.get_settings();
    Ok(serde_json::json!({ "task_parallel_max": s.task_parallel_max }))
}

#[tauri::command]
async fn task_create(
    state: tauri::State<'_, AppState>,
    title: String,
    goal: String,
    owner: Option<String>,
) -> Result<String, String> {
    state.engine.lock().await.task_create(title, goal, owner)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn task_list(state: tauri::State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let v = state.engine.lock().await.task_list().map_err(|e| e.to_string())?;
    Ok(v.into_iter()
        .map(|t| serde_json::to_value(t).unwrap_or_default())
        .map(enrich_task_json)
        .collect())
}

#[tauri::command]
async fn task_get(state: tauri::State<'_, AppState>, task_id: String) -> Result<serde_json::Value, String> {
    let t = state.engine.lock().await.task_get(&task_id).map_err(|e| e.to_string())?;
    let v = serde_json::to_value(t).map_err(|e| e.to_string())?;
    Ok(enrich_task_json(v))
}

#[tauri::command]
async fn task_plan(
    state: tauri::State<'_, AppState>,
    task_id: String,
    steps: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let steps: Vec<qibai_core::task::Step> = serde_json::from_value(serde_json::Value::Array(steps))
        .map_err(|e| format!("steps 反序列化失败：{e}"))?;
    let t = state.engine.lock().await.task_plan(&task_id, steps).map_err(|e| e.to_string())?;
    serde_json::to_value(t).map_err(|e| e.to_string())
}

#[tauri::command]
async fn task_confirm_plan(state: tauri::State<'_, AppState>, task_id: String) -> Result<serde_json::Value, String> {
    let t = state.engine.lock().await.task_confirm_plan(&task_id).map_err(|e| e.to_string())?;
    serde_json::to_value(t).map_err(|e| e.to_string())
}

#[tauri::command]
async fn task_start(state: tauri::State<'_, AppState>, task_id: String) -> Result<serde_json::Value, String> {
    let t = state.engine.lock().await.task_start(&task_id).map_err(|e| e.to_string())?;
    serde_json::to_value(t).map_err(|e| e.to_string())
}

#[tauri::command]
async fn task_pause(state: tauri::State<'_, AppState>, task_id: String) -> Result<serde_json::Value, String> {
    let t = state.engine.lock().await.task_pause(&task_id).map_err(|e| e.to_string())?;
    serde_json::to_value(t).map_err(|e| e.to_string())
}

#[tauri::command]
async fn task_resume(state: tauri::State<'_, AppState>, task_id: String) -> Result<serde_json::Value, String> {
    let t = state.engine.lock().await.task_resume(&task_id).map_err(|e| e.to_string())?;
    serde_json::to_value(t).map_err(|e| e.to_string())
}

#[tauri::command]
async fn task_cancel(state: tauri::State<'_, AppState>, task_id: String) -> Result<serde_json::Value, String> {
    let t = state.engine.lock().await.task_cancel(&task_id).map_err(|e| e.to_string())?;
    serde_json::to_value(t).map_err(|e| e.to_string())
}

#[tauri::command]
async fn task_confirm_action(
    state: tauri::State<'_, AppState>,
    task_id: String,
    action_id: String,
    allow: bool,
    remember: bool,
) -> Result<serde_json::Value, String> {
    let t = state.engine.lock().await
        .task_confirm_action(&task_id, &action_id, allow, remember)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(t).map_err(|e| e.to_string())
}

/// v45.14：Workflow 重跑。`from_step` 省略时重跑所有没成功的步；
/// 传步骤 id 时从该步起重跑（之前 Done 的步与产物保留）。
#[tauri::command]
async fn task_rerun(
    state: tauri::State<'_, AppState>,
    task_id: String,
    from_step: Option<String>,
) -> Result<serde_json::Value, String> {
    let t = state.engine.lock().await
        .task_rerun(&task_id, from_step)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(t).map_err(|e| e.to_string())
}

/// v45.10：从失败步重跑任务（保留给老前端；语义等同 task_rerun 的 from_step=None）。
#[tauri::command]
async fn task_rerun_failed(
    state: tauri::State<'_, AppState>,
    task_id: String,
) -> Result<serde_json::Value, String> {
    let t = state.engine.lock().await
        .task_rerun_failed(&task_id)
        .map_err(|e| e.to_string())?;
    serde_json::to_value(t).map_err(|e| e.to_string())
}

// ---- v45.11：知识库（kb）IPC ----

#[tauri::command]
async fn kb_add_folder(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<serde_json::Value, String> {
    let v = state.engine.lock().await.kb_add_folder(&path).map_err(|e| e.to_string())?;
    serde_json::to_value(v).map_err(|e| e.to_string())
}

#[tauri::command]
async fn kb_remove_folder(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<serde_json::Value, String> {
    let v = state.engine.lock().await.kb_remove_folder(&path).map_err(|e| e.to_string())?;
    serde_json::to_value(v).map_err(|e| e.to_string())
}

#[tauri::command]
async fn kb_reindex(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let v = state.engine.lock().await.kb_reindex().map_err(|e| e.to_string())?;
    serde_json::to_value(v).map_err(|e| e.to_string())
}

#[tauri::command]
async fn kb_status(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let v = state.engine.lock().await.kb_status().map_err(|e| e.to_string())?;
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// P2：应用内一键下载向量模型（91 MB，按需拉取，不进安装包）。
/// 下载完成立刻让 embedding 缓存失效，下一次检索就用上。
#[tauri::command]
async fn kb_embed_fetch(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let app2 = app.clone();
    let cb = move |done: u64, total: u64| {
        let pct = if total > 0 { (done.saturating_mul(100) / total) as u32 } else { 0 };
        let _ = app2.emit(
            "kb:embed_progress",
            serde_json::json!({ "done": done, "total": total, "pct": pct }),
        );
    };
    let dir = qibai_core::kb::embed::fetch_model(Some(&cb))
        .await
        .map_err(|e| e.to_string())?;
    qibai_core::kb::embed::invalidate_cache();
    let _ = app.emit("kb:embed_progress", serde_json::json!({ "done": 0, "total": 0, "pct": 100 }));
    Ok(serde_json::json!({
        "ok": true,
        "dir": dir.to_string_lossy(),
        "model": qibai_core::kb::embed::MODEL_NAME,
    }))
}

/// P2：语义检索真实状态（模型是否安装/加载、已向量化块数、融合权重）。
#[tauri::command]
async fn kb_embed_status(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let v = state.engine.lock().await.kb_embed_status();
    Ok(v)
}

#[tauri::command]
async fn kb_search(
    state: tauri::State<'_, AppState>,
    query: String,
    top_k: Option<usize>,
) -> Result<serde_json::Value, String> {
    let v = state.engine.lock().await.kb_search(&query, top_k).map_err(|e| e.to_string())?;
    serde_json::to_value(v).map_err(|e| e.to_string())
}

/// 列出可用输出设备（含 description/is_default/source，兼容旧 Vec<String>）。
#[tauri::command]
async fn list_output_devices() -> Result<Vec<serde_json::Value>, String> {
    let names = qibai_core::tts::list_output_devices();
    let current = qibai_core::tts::current_audio_output();
    Ok(names.into_iter().map(|n| {
        serde_json::json!({
            "name": n,
            "description": n,
            "is_default": n == current,
            "source": "alsa",
        })
    }).collect())
}

#[tauri::command(rename_all = "snake_case")]
async fn approve_skill(
    state: tauri::State<'_, AppState>,
    request_id: String,
    approve: bool,
) -> Result<String, String> {
    let msg = {
        let mut engine = state.engine.lock().await;
        engine
            .approve_skill(&request_id, approve)
            .await
    };
    msg.map_err(|e| {
        // v45.7：门控拒绝（如亲密度不足）真因留痕，前端会如实转述。
        eprintln!("[qibai] approve_skill err: {e}");
        e.to_string()
    })
}

// ---------- v45.12：MCP 外部工具通道 ----------
// 状态一律来自 registry 真实查询（Engine::mcp_list），绝不写死"已连接"。

/// 服务器列表：配置 + 真实状态 / 工具数 / 错误原因 / trust。
#[tauri::command(rename_all = "snake_case")]
async fn mcp_list(state: tauri::State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let mut engine = state.engine.lock().await;
    Ok(engine.mcp_list())
}

/// 增 / 改一个 server（按 name 去重，即时校验 + 落盘 + 重建注册表）。
#[tauri::command(rename_all = "snake_case")]
async fn mcp_upsert(
    state: tauri::State<'_, AppState>,
    cfg: qibai_core::mcp::McpServerConfig,
) -> Result<(), String> {
    let mut engine = state.engine.lock().await;
    engine.mcp_upsert(cfg).map_err(|e| e.to_string())
}

/// 按 name 删除一个 server。
#[tauri::command(rename_all = "snake_case")]
async fn mcp_remove(state: tauri::State<'_, AppState>, name: String) -> Result<(), String> {
    let mut engine = state.engine.lock().await;
    engine.mcp_remove(&name).map_err(|e| e.to_string())
}

/// 启停一个 server。
#[tauri::command(rename_all = "snake_case")]
async fn mcp_set_enabled(
    state: tauri::State<'_, AppState>,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let mut engine = state.engine.lock().await;
    engine.mcp_set_enabled(&name, enabled).map_err(|e| e.to_string())
}

/// 手动重连（重启该 server 子进程）。
#[tauri::command(rename_all = "snake_case")]
async fn mcp_restart(state: tauri::State<'_, AppState>, name: String) -> Result<(), String> {
    let mut engine = state.engine.lock().await;
    engine.mcp_restart(&name).map_err(|e| e.to_string())
}

/// 一条测试调用（server + tool + args）；用户在设置页主动触发，不经 trust 闸门。
#[tauri::command(rename_all = "snake_case")]
async fn mcp_test_call(
    state: tauri::State<'_, AppState>,
    server: String,
    tool: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut engine = state.engine.lock().await;
    engine.mcp_test_call(&server, &tool, args).map_err(|e| e.to_string())
}

// ---------- 开机自启 ----------

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let auto = app.autolaunch();
    if enabled {
        auto.enable().map_err(|e| e.to_string())?;
    } else {
        auto.disable().map_err(|e| e.to_string())?;
    }
    auto.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

// ---------- 契约 §2.2：qibai-core 命令补接线 ----------

#[tauri::command]
async fn get_personas(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    if let Some(v) = &state.read_cache.read().await.personas {
        return Ok(v.clone());
    }
    Ok(state.engine.lock().await.get_personas())
}

/// v45 热切换：前端从引擎读当前 active persona（窗 label 固定 pet-stage，不再从 label 推断）。
#[tauri::command]
async fn get_active_persona(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let id = state.read_cache.read().await.active_persona.clone();
    Ok(serde_json::json!({ "persona_id": id }))
}

/// 切换主角色并同步桌宠窗（v45：热切换——全程只有一个 pet-stage 窗）。
/// 引擎同步 active persona；若 stage 窗已存在，emit `persona-hot-swap` 在窗内
/// 替换立绘/音色/问候（窗口几何不变，两个人同屏在结构上不可能）；否则才建窗。
pub(crate) async fn apply_persona_switch(
    app: &tauri::AppHandle,
    engine: &Arc<Mutex<Engine>>,
    persona_id: &str,
) -> Result<(), String> {
    let t0 = std::time::Instant::now();
    {
        let mut e = engine.lock().await;
        e.set_active_persona(persona_id).map_err(|e| e.to_string())?;
    }

    let geometry = app
        .get_webview_window(STAGE_LABEL)
        .and_then(|w| w.outer_position().ok().zip(w.outer_size().ok()))
        .map(|(p, s)| format!("@({},{} {}x{})", p.x, p.y, s.width, s.height));

    if app.get_webview_window(STAGE_LABEL).is_some() {
        // 热切换：不重建窗，前端收到事件后清屏→换形象→问候。
        let _ = app.emit(
            "qibai://persona-hot-swap",
            serde_json::json!({ "persona_id": persona_id }),
        );
        eprintln!(
            "[qibai] persona-switch: HOT-Swap {} 几何={:?} 耗时{:?}",
            persona_id,
            geometry,
            t0.elapsed()
        );
    } else {
        // 首次/异常丢失：建一个隐藏窗，首帧后 show，并由前端读 active persona 画形象。
        create_pet_window(app, persona_id).map_err(|e| e.to_string())?;
        eprintln!(
            "[qibai] persona-switch: REBUILD {} 几何={:?} 耗时{:?}",
            persona_id,
            geometry,
            t0.elapsed()
        );
    }

    broadcast_desk_changed(app, engine).await;
    Ok(())
}

/// 切换主角色（命令入口）。
#[tauri::command(rename_all = "snake_case")]
async fn set_active_persona(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    persona_id: String,
) -> Result<(), String> {
    apply_persona_switch(&app, &state.engine, &persona_id).await
}

#[tauri::command(rename_all = "snake_case")]
async fn rename_persona(state: tauri::State<'_, AppState>, persona_id: String, nickname: String) -> Result<(), String> {
    state
        .engine
        .lock().await
        .rename_persona(&persona_id, &nickname)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_social_state(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    if let Some(v) = &state.read_cache.read().await.social_state {
        return Ok(v.clone());
    }
    Ok(state.engine.lock().await.get_social_state())
}

#[tauri::command]
async fn get_social_events(state: tauri::State<'_, AppState>, _limit: Option<usize>) -> Result<serde_json::Value, String> {
    // 快照里已按 limit=50 预取；limit 原样参数被忽略（UI 事件列表长度变化对只读非关键）。
    if let Some(v) = &state.read_cache.read().await.social_events {
        return Ok(v.clone());
    }
    Ok(state.engine.lock().await.get_social_events(_limit))
}

// ---------- 契约 §2.3：窗口管理命令 ----------

/// 返回调用方窗口身份：`{label}`（前端从 `pet-<id>` 解析 persona_id）。
#[tauri::command]
fn window_identity(window: tauri::WebviewWindow) -> serde_json::Value {
    // 诊断：前端是否真正走 Tauri invoke（若未见此行 = 前端在 mock 模式）
    eprintln!("[qibai] window_identity called (frontend-bridge=real)");
    serde_json::json!({ "label": window.label() })
}

#[tauri::command]
async fn get_desk_state(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    // v45.3：桌宠窗后台高频轮询的接口——长任务跑时不能卡。
    if let Some(v) = &state.read_cache.read().await.desk_state {
        return Ok(v.clone());
    }
    Ok(state.engine.lock().await.desk_state())
}

#[tauri::command(rename_all = "snake_case")]
async fn pet_summon(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    persona_id: String,
) -> Result<(), String> {
    {
        let mut engine = state.engine.lock().await;
        engine
            .desk_summon(&persona_id)
            .map_err(|e| e.to_string())?;
    }
    let label = STAGE_LABEL;
    if let Some(w) = app.get_webview_window(label) {
        // v45：stage 窗已存在 → 热切换到目标 persona + 显示，不再新建第二个窗。
        let _ = w.show();
        let _ = w.set_focus();
        let _ = app.emit(
            "qibai://persona-hot-swap",
            serde_json::json!({ "persona_id": persona_id }),
        );
    } else {
        create_pet_window(&app, &persona_id).map_err(|e| e.to_string())?;
    }
    broadcast_desk_changed(&app, &state.engine).await;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
async fn pet_dismiss(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    persona_id: String,
) -> Result<(), String> {
    {
        let mut engine = state.engine.lock().await;
        engine
            .desk_dismiss(&persona_id)
            .map_err(|e| e.to_string())?;
    }
    let label = STAGE_LABEL;
    if let Some(w) = app.get_webview_window(&label) {
        // v45：热切换单窗——dismiss 只隐藏 stage（保留几何），不销毁重建。
        let _ = w.hide();
    }
    broadcast_desk_changed(&app, &state.engine).await;
    Ok(())
}

#[tauri::command(rename_all = "snake_case")]
async fn pet_set_locked(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    persona_id: String,
    locked: bool,
) -> Result<(), String> {
    state
        .engine
        .lock().await
        .desk_set_locked(&persona_id, locked)
        .map_err(|e| e.to_string())?;
    broadcast_desk_changed(&app, &state.engine).await;
    Ok(())
}

#[tauri::command]
fn console_open(app: tauri::AppHandle) -> Result<(), String> {
    ensure_console(&app).map_err(|e| e.to_string())
}

/// v21.11.8：控制台关闭命令（前端 console.js/pet.js 此前调 close_console/console_close
/// 均未注册 → 静默失败，「收起控制台」失效）。hide 而非 close：保留窗口复用。
#[tauri::command]
fn console_close(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("console") {
        let _ = w.hide();
    }
    Ok(())
}

/// 语音 UI 指令分发（前端文本框「指令直达」入口）：
/// 与 B 线语音链路共用 qibai-core::voice_ui_cmd 匹配表 + listener::run_voice_ui_cmd 执行器。
/// 返回 `{matched: bool, reply: String}`；未命中 matched=false reply=""（前端继续走普通聊天）。
#[tauri::command]
async fn voice_ui_dispatch(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    text: String,
) -> Result<serde_json::Value, String> {
    match qibai_core::voice_ui_cmd::match_ui_command(&text) {
        None => Ok(serde_json::json!({ "matched": false, "reply": "" })),
        Some(m) => {
            let reply = listener::run_voice_ui_cmd(&app, &state.engine, m).await;
            // v21.11.8：命令命中 → ①写会话（控制台可见记录）②后端直接播放回复。
            // 旧链路：前端 say(reply) 走 tts_speak，被「窗口聚焦静音门」卡住——
            // PTT 按住说话时窗口聚焦 → 命令类回复（查天气等）只显气泡不出声；
            // 且不写会话 → 控制台看不到记录。现与 LLM 流式同路径（后端播放 + AEC 保护）。
            let persona_id = {
                let e = state.engine.lock().await;
                e.personas.active_persona().to_string()
            };
            {
                let mut e = state.engine.lock().await;
                let _ = e.append_chat("user", &text);
                let _ = e.append_chat("assistant", &reply);
            }
            let mut rust_played = false;
            if !reply.trim().is_empty() {
                let req = {
                    let e = state.engine.lock().await;
                    e.tts_decide(tts::TtsKind::Chat, &reply, Some(&persona_id))
                };
                if let Some(req) = req {
                    let r = perform_speech(&app, &state.engine, req).await;
                    rust_played = r.get("engine").and_then(|v| v.as_str()) != Some("mute");
                }
            }
            Ok(serde_json::json!({
                "matched": true,
                "reply": reply,
                "rust_played": rust_played,
            }))
        }
    }
}

#[tauri::command]
fn pets_show(app: tauri::AppHandle) -> Result<(), String> {
    for (_, w) in app.webview_windows() {
        if w.label().starts_with("pet-") {
            let _ = w.show();
        }
    }
    Ok(())
}

#[tauri::command]
fn pets_hide(app: tauri::AppHandle) -> Result<(), String> {
    for (_, w) in app.webview_windows() {
        if w.label().starts_with("pet-") {
            let _ = w.hide();
        }
    }
    Ok(())
}

#[tauri::command]
fn app_exit(app: tauri::AppHandle) -> Result<(), String> {
    app.exit(0);
    Ok(())
}

/// 桌宠页内容就绪报信（渲染兜底心跳）：前端 pet.html 加载完成后调用。
#[tauri::command]
fn pet_ready(state: tauri::State<'_, AppState>) {
    state.pet_ready.store(true, Ordering::SeqCst);
}

// ---------- 契约 §2.4：社交命令包装（后端协调跨窗口演出） ----------

/// 串门寒暄：调引擎 social_greet(from, to, false)，台词逐条广播 `qibai://pet-say`。
#[tauri::command]
async fn social_greet(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    from: String,
    to: String,
) -> Result<serde_json::Value, String> {
    let out = {
        let mut engine = state.engine.lock().await;
        engine.social_greet(&from, &to, false).await
    };
    match out {
        Ok(v) => {
            if let Some(lines) = v["lines"].as_array() {
                broadcast_pet_say_lines(&app, lines);
            }
            Ok(v)
        }
        Err(e) => Err(e.to_string()),
    }
}

/// 猜拳：调引擎 social_pk；广播 `qibai://pk-round`（含双方手势与 lines）；
/// 非 blocked 时逐条广播 `qibai://pet-say`；返回引擎结果。
#[tauri::command(rename_all = "snake_case")]
async fn social_pk(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    a: String,
    b: String,
    move_a: String,
    move_b: String,
) -> Result<serde_json::Value, String> {
    let v = {
        let mut engine = state.engine.lock().await;
        engine.social_pk(&a, &b, &move_a, &move_b)
    };
    match v {
        Ok(v) => {
            let blocked = v["blocked"].as_bool().unwrap_or(false);
            let _ = app.emit(
                "qibai://pk-round",
                serde_json::json!({
                    "a": a,
                    "b": b,
                    "move_a": move_a,
                    "move_b": move_b,
                    "result": v["result"].clone(),
                    "winner": v["winner"].clone(),
                    "lines": v["lines"].clone(),
                }),
            );
            if !blocked {
                if let Some(lines) = v["lines"].as_array() {
                    broadcast_pet_say_lines(&app, lines);
                }
            }
            Ok(v)
        }
        Err(e) => Err(e.to_string()),
    }
}

// ---------- 契约 §2.5：亲密度命令（冻结规格 docs/INTIMACY_SYSTEM.md §6） ----------

#[tauri::command]
async fn get_intimacy_levels(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    if let Some(v) = &state.read_cache.read().await.intimacy_levels {
        return Ok(v.clone());
    }
    Ok(state.engine.lock().await.get_intimacy_levels())
}

#[tauri::command(rename_all = "snake_case")]
async fn get_intimacy_state(
    state: tauri::State<'_, AppState>,
    persona_id: String,
) -> Result<serde_json::Value, String> {
    state
        .engine
        .lock().await
        .get_intimacy_state(&persona_id)
        .map_err(|e| e.to_string())
}

/// 亲密度事件：成功后 app-wide 广播 `qibai://intimacy-updated`（§6.2，
/// 控制台刷新亲密度页 / 桌宠窗 leveled_up=true 时播庆祝动效）。
/// 迭代②：leveled_up=true 时朗读升级台词（仅 All scope；失败静默降级）。
#[tauri::command(rename_all = "snake_case")]
async fn intimacy_event(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    persona_id: String,
    kind: String,
    detail: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let detail = detail.unwrap_or_default();
    let out = {
        let mut engine = state.engine.lock().await;
        engine
            .intimacy_event(&persona_id, &kind, detail)
            .map_err(|e| e.to_string())?
    };
    // 广播 payload 形状（§6.2）：points/level/level_name 取自最新状态。
    let st = state
        .engine
        .lock().await
        .get_intimacy_state(&persona_id)
        .unwrap_or(serde_json::Value::Null);
    let _ = app.emit(
        "qibai://intimacy-updated",
        serde_json::json!({
            "persona_id": persona_id,
            "points": st["points"].clone(),
            "level": st["level"].clone(),
            "level_name": st["level_name"].clone(),
            "leveled_up": out["leveled_up"].clone(),
            "unlocked": out["unlocked"].clone(),
            "achievements_granted": out["achievements_granted"].clone(),
        }),
    );

    // 迭代②：升级台词朗读（仅 All scope；门控与降级在 tts_decide/perform_speech 内）
    if out["leveled_up"] == serde_json::Value::Bool(true) {
        let line = format!(
            "亲密度提升啦！现在是 L{} {}，要继续一起走下去呀～",
            out["level"],
            st["level_name"].as_str().unwrap_or("")
        );
        if let Some(req) = state
            .engine
            .lock().await
            .tts_decide(tts::TtsKind::LevelUp, &line, Some(persona_id.as_str()))
        {
            // P0-B 调优：升级台词同样过投递窗口，防误 barge-in 后后台口播自说自话。
            if qibai_core::tts::delivery_window_now().is_safe() {
                perform_speech(&app, &state.engine, req).await;
            } else {
                eprintln!("[qibai] level-up 台词: 投递窗口不安全→ 本轮跳过口播");
            }
        }
    }
    Ok(out)
}

#[tauri::command(rename_all = "snake_case")]
async fn set_anniversary(
    state: tauri::State<'_, AppState>,
    persona_id: String,
    month: u8,
    day: u8,
) -> Result<(), String> {
    state
        .engine
        .lock().await
        .set_anniversary(&persona_id, month, day)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
async fn clear_anniversary(
    state: tauri::State<'_, AppState>,
    persona_id: String,
) -> Result<(), String> {
    state
        .engine
        .lock().await
        .clear_anniversary(&persona_id)
        .map_err(|e| e.to_string())
}

/// v18.8-fix8：语音诊断实时数据（前端每 1s 轮询；说话时看 rms 跳不跳、
/// vad_count 涨不涨、last_text 是否转出文本——一次看清链路断在哪）。
/// v20：新增 stt_engine 段（server/cli/model 分开命中 + 放置指引）与
/// tts_playback 段（无声归因二分：playing/done/failed + 引擎名 + 时长 + 错误）。
#[tauri::command]
fn voice_diag(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let l = &state.listen;
    let stt = qibai_core::whisper_engine::stt_engine_status();
    let pb = qibai_core::tts::current_playback_status();
    Ok(serde_json::json!({
        "enabled": l.enabled.load(std::sync::atomic::Ordering::SeqCst),
        "state": l.state.load(std::sync::atomic::Ordering::SeqCst),
        "device": l.diag.device.lock().unwrap().clone(),
        "rate": *l.diag.rate.lock().unwrap(),
        "rms": *l.diag.rms.lock().unwrap(),
        "vad_count": l.diag.vad_count.load(std::sync::atomic::Ordering::SeqCst),
        "last_text": l.diag.last_text.lock().unwrap().clone(),
        "stt_engine": {
            "server_found": stt.server_found,
            "server_path": stt.server_path,
            "cli_found": stt.cli_found,
            "cli_path": stt.cli_path,
            "model_found": stt.model_found,
            "model_path": stt.model_path,
            "probe_ok": stt.probe_ok,
            "probe_reason": stt.probe_reason,
            "cwd": stt.cwd,
            "placement_guide": stt.placement_guide,
            "download_links": stt.download_links,
        },
        // P0 回声根治：AEC 启用状态（env QIBAI_AEC=0 → false），真机验证一眼可查。
        "aec_enabled": qibai_core::aec::aec_enabled(),
        "tts_playback": {
            "engine": pb.engine,
            "state": pb.state,
            "duration_ms": pb.duration_ms,
            "last_error": pb.last_error,
        },
    }))
}

#[tauri::command(rename_all = "snake_case")]
async fn check_gate(
    state: tauri::State<'_, AppState>,
    persona_id: String,
    gate: String,
) -> Result<serde_json::Value, String> {
    state
        .engine
        .lock().await
        .check_gate(&persona_id, &gate)
        .map_err(|e| e.to_string())
}

// ---------- 迭代①：全屏守护演出 + 代码检查 ----------

/// 当前 Unix 秒（久坐 tick 透传给守护调度器）。
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 极简 base64 编码（不引入新依赖）。
fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// 解析标准 RIFF/WAV 头的时长（秒）。byteRate 在偏移 28，data chunk size 在偏移 40。
/// 非标准布局或不足 44 字节返回 0（调用方忽略）。
fn wav_duration_secs(wav: &[u8]) -> f64 {
    if wav.len() < 44 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return 0.0;
    }
    let byte_rate = u32::from_le_bytes([wav[28], wav[29], wav[30], wav[31]]) as f64;
    if byte_rate <= 0.0 {
        return 0.0;
    }
    let data_len = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as f64;
    (data_len / byte_rate).max(0.5) // 至少 0.5s，防短音频窗口太窄
}

/// 截图捕获：按三平台命令抓桌面到临时 PNG → base64 data URL。
/// 全部失败返回 None（前端用 CSS 模拟桌面，纯视觉）。真机验证项。
fn capture_screenshot(_app: &tauri::AppHandle) -> Option<String> {
    let tmp = std::env::temp_dir().join("qibai_guard_shot.png");
    let _ = std::fs::remove_file(&tmp);

    macro_rules! try_run {
        ($prog:expr, $args:expr) => {
            std::process::Command::new($prog)
                .args($args.iter().copied())
                .arg(&tmp)
                .output()
                .ok()
        };
    }

    #[cfg(target_os = "linux")]
    {
        // 依次尝试 ImageMagick import / gnome-screenshot / scrot
        let _ = try_run!("import", &["-window", "root", "-silent"]);
        if !tmp.exists() {
            let _ = try_run!("gnome-screenshot", &["-f"]);
        }
        if !tmp.exists() {
            let _ = try_run!("scrot", &[] as &[&str]);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = try_run!("screencapture", &["-x"]);
    }
    #[cfg(target_os = "windows")]
    {
        let path = tmp.to_string_lossy().replace('\\', "\\\\");
        let ps = format!(
            "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
             $b = New-Object System.Drawing.Bitmap([System.Windows.Forms.Screen]::PrimaryScreen.Bounds.Width, \
             [System.Windows.Forms.Screen]::PrimaryScreen.Bounds.Height); \
             $g = [System.Drawing.Graphics]::FromImage($b); \
             $g.CopyFromScreen(0,0,0,0,$b.Size); \
             $b.Save('{}'); $g.Dispose(); $b.Dispose();",
            path
        );
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .output();
    }

    let bytes = std::fs::read(&tmp).ok()?;
    let _ = std::fs::remove_file(&tmp);
    if bytes.is_empty() {
        return None;
    }
    Some(format!("data:image/png;base64,{}", base64_encode(&bytes)))
}

/// 守护演出共用 helper：创建/复用全屏 guard 窗 → 组 payload（含 persona_id / 截图）
/// → 广播 `qibai://guard-start`。返回 payload（供命令原样回给前端）。
/// 迭代②：全屏演出时朗读开场语（仅 All scope；失败静默降级）。
async fn start_guard_performance(
    app: &tauri::AppHandle,
    engine: &Arc<Mutex<Engine>>,
    start: qibai_core::guard::GuardStart,
) -> serde_json::Value {
    if app.get_webview_window("guard").is_none() {
        let _ = WebviewWindowBuilder::new(
            app,
            "guard",
            WebviewUrl::App("guard.html".into()),
        )
        .fullscreen(true)
        .always_on_top(true)
        .decorations(false)
        .resizable(false)
        .shadow(false)
        .build();
    } else if let Some(w) = app.get_webview_window("guard") {
        let _ = w.show();
        let _ = w.set_focus();
    }

    let persona_id = engine
        .lock().await
        .personas
        .active_persona()
        .to_string();

    let mut payload = serde_json::json!({
        "mode": start.mode.as_str(),
        "hours": start.hours,
        "message": start.message,
        "end_action": start.end_action.as_str(),
        "countdown_secs": start.countdown_secs,
        "perform_fullscreen": start.perform_fullscreen,
        "persona_id": persona_id,
    });
    if let Some(shot) = capture_screenshot(app) {
        payload["screenshot"] = serde_json::Value::String(shot);
    }
    let _ = app.emit("qibai://guard-start", payload.clone());

    // 迭代②：全屏演出才朗读开场语（仅 All scope；门控与降级在 tts_decide/perform_speech 内）
    // 迭代⑤：开场语用当前主角色的专属音色（persona_id 本函数内已解析）。
    if start.perform_fullscreen {
        let message = start.message.clone();
        if let Some(req) = engine
            .lock().await
            .tts_decide(tts::TtsKind::Guard, &message, Some(persona_id.as_str()))
        {
            // P0-B 调优：开场语也过投递窗口——误 barge-in 挂起口播后，
            // 后台事件不应直接抢播（否则间接助长「自说自话」观感）。
            if qibai_core::tts::delivery_window_now().is_safe() {
                perform_speech(app, engine, req).await;
            } else {
                eprintln!("[qibai] guard 开场语: 投递窗口不安全→ 本轮跳过口播");
            }
        }
    }
    payload
}

/// 手动触发一次全屏守护演出。未启用 / 冷却中 → Err。
#[tauri::command]
async fn guard_trigger(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let start = {
        let mut engine = state.engine.lock().await;
        engine.guard_manual_trigger()
    };
    match start {
        None => Err("守护当前不可用（未启用或冷却中）".to_string()),
        Some(start) => Ok(start_guard_performance(&app, &state.engine, start).await),
    }
}

/// 跳过演出：关闭 guard 窗并广播 `qibai://guard-end`（不执行收尾动作）。
#[tauri::command]
async fn guard_skip(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    reason: String,
) -> Result<String, String> {
    let end_action = {
        let mut engine = state.engine.lock().await;
        engine.guard_skip(&reason).map_err(|e| e.to_string())?;
        engine.settings.guard.end_action.as_str().to_string()
    };
    if let Some(w) = app.get_webview_window("guard") {
        let _ = w.close();
    }
    let _ = app.emit(
        "qibai://guard-end",
        serde_json::json!({
            "skipped": reason == "user",
            "end_action": end_action,
        }),
    );
    Ok("ok".to_string())
}

/// 执行收尾系统动作（息屏 / 锁屏）。沙箱/无权限环境走 simulated 分支（预期降级）。
#[tauri::command]
async fn guard_system_action(
    app: tauri::AppHandle,
    action: String,
) -> Result<serde_json::Value, String> {
    let cmd = match action.as_str() {
        "sleep" => qibai_core::guard::platform_sleep_cmd(),
        "lock" => qibai_core::guard::platform_lock_cmd(),
        other => return Err(format!("unknown guard action: {other}")),
    };
    let argv: Vec<&str> = cmd.split_whitespace().collect();
    let (simulated, detail) = match argv.is_empty() {
        true => (true, format!("环境不支持，已模拟：{cmd}（空命令）")),
        false => match std::process::Command::new(argv[0]).args(&argv[1..]).status() {
            Ok(_) => (false, format!("已执行 {cmd}")),
            Err(e) => (true, format!("环境不支持，已模拟：{cmd}（{e}）")),
        },
    };
    let _ = app.emit(
        "qibai://guard-action",
        serde_json::json!({
            "action": action,
            "simulated": simulated,
            "detail": detail,
        }),
    );
    Ok(serde_json::json!({ "simulated": simulated, "detail": detail }))
}

/// 代码检查：path / snippet 二选一（经 codecheck::CheckRequest::from_json 校验）。
#[tauri::command]
async fn check_code(
    state: tauri::State<'_, AppState>,
    path: Option<String>,
    snippet: Option<String>,
    language: Option<String>,
) -> Result<serde_json::Value, String> {
    let path = path.and_then(|p| {
        let t = p.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    });
    let snippet = snippet.and_then(|s| {
        let t = s.trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    });
    let source = match (&path, &snippet) {
        (Some(_), None) => "path",
        (None, Some(_)) => "snippet",
        (Some(_), Some(_)) => {
            return Err("path 与 snippet 只能提供一个".to_string());
        }
        (None, None) => {
            return Err("需要提供 path（文件/文件夹）或 snippet（代码片段）".to_string());
        }
    };
    let mut v = serde_json::json!({ "source": source });
    if let Some(p) = &path {
        v["path"] = serde_json::json!(p);
    }
    if let Some(s) = &snippet {
        v["snippet"] = serde_json::json!(s);
    }
    if let Some(l) = &language {
        v["language"] = serde_json::json!(l);
    }
    let req = qibai_core::codecheck::CheckRequest::from_json(&v).map_err(|e| e)?;

    let report = {
        let mut engine = state.engine.lock().await;
        engine.check_code(req).await
    };
    match report {
        Ok(r) => serde_json::to_value(r).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

// ---------- 迭代②：TTS 发声陪伴 ----------

use qibai_core::tts::{self, ResolvedEngine};
use qibai_core::whisper_engine;
use qibai_core::TtsSpeakRequest;

/// 成功朗读路径向前端发 `qibai://tts-speak` 指示器（Mute 不发）。
fn emit_speaking_indicator(app: &tauri::AppHandle, req: &TtsSpeakRequest, engine_tag: &str) {
    let _ = app.emit(
        "qibai://tts-speak",
        serde_json::json!({
            "kind": req.kind,
            "engine": engine_tag,
            "volume": req.volume,
            "rate": req.rate,
            "state": "speaking",
        }),
    );
}

/// 云端失败后的降级：本地可用则本地播放（degraded=true），否则结构化静音结果。
async fn fallback_local_or_mute(
    app: &tauri::AppHandle,
    _engine: &Arc<Mutex<Engine>>,
    req: &TtsSpeakRequest,
    err: &str,
) -> serde_json::Value {
    if tts::detect_local_tts_available() {
        // 迭代⑤：本地 TTS 调用补 pitch / engine_voice_id 参数（per-角色音色）。
        match tts::run_local_tts(&req.text, req.rate, req.volume, req.pitch, &req.engine_voice_id) {
            Ok(true) => {
                emit_speaking_indicator(app, req, "local");
                serde_json::json!({
                    "engine": "local",
                    "played": true,
                    "degraded": true,
                    "detail": format!("云端 TTS 失败（{err}），已降级本地播放"),
                })
            }
            _ => serde_json::json!({
                "engine": "mute",
                "played": false,
                "degraded": true,
                "reason": format!("云端 TTS 失败（{err}），且本地 TTS 不可用"),
            }),
        }
    } else {
        serde_json::json!({
            "engine": "mute",
            "played": false,
            "degraded": true,
            "reason": format!("云端 TTS 失败：{err}，且无本地 TTS 可用"),
        })
    }
}

/// 朗读共用 helper（迭代②）：按 req.resolved 分派本地/云端，成功路径发指示器。
/// 任何失败都降级为结构化 JSON，绝不 panic、绝不向上抛错。
async fn perform_speech(
    app: &tauri::AppHandle,
    engine: &Arc<Mutex<Engine>>,
    req: TtsSpeakRequest,
) -> serde_json::Value {
    // v20 出声链路锚点：入口统一打（决策结果 + 文本长度，绝不打文本），便于真机 grep [qibai-voice]。
    eprintln!(
        "[qibai-voice] perform_speech entry resolved={:?} reason='{}' text_len={}",
        req.resolved, req.reason, req.text.chars().count()
    );
    match req.resolved {
        ResolvedEngine::Local => match tts::run_local_tts(&req.text, req.rate, req.volume, req.pitch, &req.engine_voice_id) {
            Ok(true) => {
                eprintln!("[qibai-voice] perform_speech result=played(engine=local)");
                tts::record_spoken(&req.text);
                emit_speaking_indicator(app, &req, "local");
                serde_json::json!({
                    "engine": "local",
                    "played": true,
                    "degraded": false,
                    "detail": "本地 TTS 已播放",
                })
            }
            Ok(false) => {
                eprintln!("[qibai-voice] perform_speech result=mute(本地引擎全不可用)");
                serde_json::json!({
                    "engine": "mute",
                    "played": false,
                    "degraded": true,
                    "reason": "本地 TTS 不可用",
                })
            }
            Err(e) => {
                eprintln!("[qibai-voice] perform_speech result=failed(本地启动: {e})");
                serde_json::json!({
                    "engine": "mute",
                    "played": false,
                    "degraded": true,
                    "reason": format!("本地 TTS 启动失败: {e}"),
                })
            }
        },
        ResolvedEngine::Cloud => {
            // 云端请求构造与合成失败一律降级，绝不向调用方抛错。
            // 迭代⑤：云端 speaker 用角色专属 cloud_speaker（空串时 core 内回落到 settings.voice.cloud_voice）。
            let voice = engine.lock().await.settings.voice.clone();
            let creq = match tts::build_cloud_request_with_speaker(&voice, &req.text, &req.cloud_speaker) {
                Ok(r) => r,
                Err(e) => return fallback_local_or_mute(app, engine, &req, &e.to_string()).await,
            };
            match tts::synth_cloud(creq).await {
                Ok(bytes) => {
                    // P0 回声根治：云端 TTS 由前端 <audio> 播放（绕过 rodio 队列），
                    // 参考信号在此 tap——发 data URL 给前端前把 wav 字节送 AEC 参考环。
                    qibai_core::aec::ingest_reference_wav(&bytes);
                    // P0-B：云端播放无 rodio 线程能量源，wav 字节解码 RMS 补播放能量环。
                    qibai_core::tts::echo_gate_note_wav_bytes(&bytes);
                    // P0-C（回声根治闭环）：前端 <audio> 播放不走 rodio，TTS_SPEAKING 静音门
                    // 从未置位 → 播放时麦克风不静音 → 残响转写 → 自说自话。
                    // 按 wav 实际时长置位（+400ms 尾音窗由 mark_speaking_for 内部处理）。
                    let dur = wav_duration_secs(&bytes);
                    if dur > 0.0 {
                        qibai_core::tts::mark_speaking_for(std::time::Duration::from_secs_f64(dur));
                        eprintln!("[qibai-voice] cloud 播放置位静音门 dur={:.2}s", dur);
                    }
                    // v45.8：若用户选了非默认输出设备，改走 rodio/pacat 播放（不发 data_url），
                    // 否则 <audio> 永远走系统默认，用户切了等于没切。
                    let current_dev = qibai_core::tts::current_audio_output();
                    let cloud_json = if !current_dev.is_empty() && current_dev != "default" {
                        eprintln!("[qibai-voice] cloud 走设备 {current_dev} 播放（非 <audio>）");
                        qibai_core::tts::play_audio_bytes(&bytes, "cloud").ok();
                        emit_speaking_indicator(app, &req, "cloud");
                        serde_json::json!({
                            "engine": "cloud",
                            "played": true,
                            "degraded": false,
                            "detail": format!("云端 TTS 已通过 {current_dev} 播放"),
                        })
                    } else {
                        let data_url = format!("data:audio/wav;base64,{}", base64_encode(&bytes));
                        eprintln!("[qibai-voice] perform_speech result=cloud_synth_ok bytes={}", bytes.len());
                        emit_speaking_indicator(app, &req, "cloud");
                        serde_json::json!({
                            "engine": "cloud",
                            "played": false,
                            "degraded": false,
                            "audio_data_url": data_url,
                            "detail": "云端 TTS 合成成功",
                        })
                    };
                    cloud_json
                }
                Err(e) => return fallback_local_or_mute(app, engine, &req, &e.to_string()).await,
            }
        }
        ResolvedEngine::Mute => {
            eprintln!("[qibai-voice] perform_speech result=mute reason='{}'", req.reason);
            serde_json::json!({
                "engine": "mute",
                "played": false,
                "degraded": false,
                "reason": req.reason,
            })
        }
    }
}

/// 前端主动触发朗读（喇叭重读 / 对话回复朗读）。None 决策 → 结构化静音结果。
/// v16：扁平参数（匹配前端调用点），不再用 payload 包裹。
#[tauri::command(rename_all = "snake_case")]
async fn tts_speak(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    text: String,
    kind: String,
    persona_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let kind = tts::TtsKind::parse(&kind).unwrap_or(tts::TtsKind::Chat);
    // 隐私：绝不打印朗读原文，只打字数。
    eprintln!("[qibai] tts_speak: kind={:?} text_len={} persona={:?}", kind, text.chars().count(), persona_id);
    let req = state
        .engine
        .lock().await
        .tts_decide(kind, &text, persona_id.as_deref());
    match req {
        None => Ok(serde_json::json!({
            "engine": "mute",
            "played": false,
            "degraded": false,
            "reason": "朗读未启用或范围未覆盖",
        })),
        Some(req) => Ok(perform_speech(&app, &state.engine, req).await),
    }
}

/// 迭代⑤：更新某角色的专属 voice_profile（逐角色音色）。
/// 扁平参数：{ persona_id, voice_profile: {mode,pitch,rate,engine_voice_id,cloud_speaker} }。
/// voice_profile 以 serde_json::Value 传入，core 内做 clamp/校验与持久化。
#[tauri::command(rename_all = "snake_case")]
async fn set_persona_voice(
    state: tauri::State<'_, AppState>,
    persona_id: String,
    voice_profile: serde_json::Value,
) -> Result<(), String> {
    let profile: qibai_core::voice_profile::VoiceProfile =
        serde_json::from_value(voice_profile).map_err(|e| e.to_string())?;
    state
        .engine
        .lock().await
        .set_persona_voice(&persona_id, profile)
        .map_err(|e| e.to_string())
}

/// 当前 TTS 引擎状态（设置页语音状态提示）。
#[tauri::command]
async fn tts_engine_status(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let local_available = tts::detect_local_tts_available();
    Ok(state.engine.lock().await.tts_status(local_available))
}

/// 当前 STT 状态（设置页降级提示；无 key / 无设备明确提示，不报错）。
#[tauri::command]
async fn stt_status(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    if let Some(v) = &state.read_cache.read().await.stt_status {
        return Ok(v.clone());
    }
    Ok(state.engine.lock().await.stt_status())
}

/// 语音转写：audio_b64 = 前端录音的 base64 音频（wav 16k 最稳）。
#[tauri::command(rename_all = "snake_case")]
async fn stt_transcribe(
    state: tauri::State<'_, AppState>,
    audio_b64: String,
) -> Result<String, String> {
    state.engine.lock().await.stt_transcribe(&audio_b64).await
}

/// v11 PTT：开始 Rust 直采麦克风（按住说话，不依赖 WebView getUserMedia 权限）。
#[tauri::command]
fn ptt_start() -> Result<serde_json::Value, String> {
    ptt::ptt_start()?;
    Ok(serde_json::json!({"ok": true}))
}

/// v11 PTT：停止采集并返回 16k 单声道 WAV base64（不足 0.1s 返回空串）。
#[tauri::command]
fn ptt_stop() -> Result<serde_json::Value, String> {
    let b64 = ptt::ptt_stop()?;
    Ok(serde_json::json!({"audio_b64": b64}))
}

/// 输出设备自检：播 1s 测试音并抓 `pactl list sink-inputs` 的实际落点。
/// 原来前端点了 `audio_test_output` 但命令根本没注册，按钮必然报找不到命令。
#[tauri::command(rename_all = "snake_case")]
async fn audio_test_output(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    state
        .engine
        .lock()
        .await
        .audio_test_output()
        .map_err(|e| e.to_string())
}

/// 只探测不发声（排障时想先看清后端与工具链）。
#[tauri::command(rename_all = "snake_case")]
async fn audio_diagnose(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    state
        .engine
        .lock()
        .await
        .audio_diagnose()
        .map_err(|e| e.to_string())
}

/// 打开用户技能目录（`<数据目录>/plugins/skills`），跨平台唤起资源管理器。
/// 原前端调 `open_workspace_dir` 但没这个命令，catch 里只能吐一句写死的
/// `~/.qibai_skills`——既没打开目录，路径也是错的。
#[tauri::command(rename_all = "snake_case")]
fn skill_open_dir() -> Result<String, String> {
    let root = qibai_core::skill_install::user_skills_root();
    let _ = std::fs::create_dir_all(&root);
    let shown = root.display().to_string();
    if open_path(&shown) {
        Ok(format!("已打开技能目录：{shown}"))
    } else {
        Ok(format!("技能目录：{shown}（未能自动打开，请手动进入）"))
    }
}

/// 用系统默认程序打开路径或 URL（`open_external` / `skill_open_dir` 共用）。
fn open_path(target: &str) -> bool {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", target]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(target);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(target);
        c
    };
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}

/// 打开外部链接（聊天里出现的来源 URL）。原先该命令不存在，`pet.js` 只能落回
/// `window.open`，在 Tauri 里会开一个空白 WebView。
#[tauri::command(rename_all = "snake_case")]
fn open_external(url: String) -> Result<(), String> {
    let u = url.trim();
    if !(u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:")) {
        return Err("只允许 http/https/mailto 链接".to_string());
    }
    if open_path(u) {
        Ok(())
    } else {
        Err(format!("无法唤起系统浏览器：{u}"))
    }
}

/// v20：播放测试音自检——合成 0.5s 880Hz 正弦 wav 走全局 rodio 播放管道，
/// 用于真机定位「输出设备是否出声」。返回实际时长/错误（无声时明确归因）。
#[tauri::command]
fn tts_test_tone() -> Result<serde_json::Value, String> {
    let rate: u32 = 16000;
    let dur_secs: f32 = 0.5;
    let n = (rate as f32 * dur_secs) as usize;
    let mut samples: Vec<i16> = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / rate as f32;
        let v = (2.0 * std::f32::consts::PI * 880.0 * t).sin() * 0.3;
        samples.push((v * 32767.0) as i16);
    }
    // 拼最小 WAV 头（16kHz mono 16-bit）
    let data_len = (n * 2) as u32;
    let mut wav: Vec<u8> = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36u32 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());          // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes());           // PCM
    wav.extend_from_slice(&1u16.to_le_bytes());           // mono
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());      // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes());           // block align
    wav.extend_from_slice(&16u16.to_le_bytes());           // bits
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for s in &samples {
        wav.extend_from_slice(&s.to_le_bytes());
    }
    match tts::play_audio_bytes(&wav, "test-tone") {
        Ok(ms) => {
            tts::clear_speaking();
            Ok(serde_json::json!({"played": true, "duration_ms": ms}))
        }
        Err(e) => Ok(serde_json::json!({"played": false, "error": e})),
    }
}

// ---------- 迭代③：本地密码保险箱（冻结契约 docs/VAULT_SPEC.md §4） ----------
// 红线（§7）：密码明文只在 core 内存 + 本层写剪贴板的瞬时存在；
// vault_copy 返回给前端的载荷不含明文；日志/错误一律不带明文。

/// 钥匙串条目标识：identifier=com.qibai.app，account=qibai-vault。
const KEYRING_SERVICE: &str = "com.qibai.app";
/// 真实保险箱主密钥在钥匙串中的 account 名。
const KEYRING_USER: &str = "qibai-vault";
/// 可用性探测专用 account（set→get→del，绝不触碰真实 key）。
const KEYRING_PROBE_USER: &str = "qibai-vault-probe";

/// 极简 hex 编码（32B key 以 hex 字符串存钥匙串，不引入新依赖）。
fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// hex 解码（返回 None 表示格式损坏）。
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i < b.len() {
        let hi = (b[i] as char).to_digit(16)?;
        let lo = (b[i + 1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
        i += 2;
    }
    Some(out)
}

/// CSPRNG 生成 32B 随机主密钥（契约 §1：keyring 模式 key = 系统钥匙串随机主密钥）。
fn random_master_key() -> Result<[u8; 32], String> {
    let mut key = [0u8; 32];
    getrandom::getrandom(&mut key).map_err(|e| format!("无法生成随机主密钥（{e}）"))?;
    Ok(key)
}

/// 探测系统钥匙串是否可用：探针条目 set→get→del 全成功才算可用。
/// 沙箱无 Credential Manager / Keychain / Secret Service，此处走查验证，真机确认。
fn keyring_available() -> bool {
    let entry = match keyring::Entry::new(KEYRING_SERVICE, KEYRING_PROBE_USER) {
        Ok(e) => e,
        Err(_) => return false,
    };
    if entry.set_password("qibai-probe").is_err() {
        return false;
    }
    let ok = entry
        .get_password()
        .map(|v| v == "qibai-probe")
        .unwrap_or(false);
    // 清理探针（删除失败不影响探测结论）
    let _ = entry.delete_credential();
    ok
}

/// 从钥匙串读 32B key。错误统一转中文提示（不暴露系统细节到前端文案之外）。
fn keyring_get() -> Result<Vec<u8>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("系统钥匙串不可用（{e}）"))?;
    let stored = entry
        .get_password()
        .map_err(|e| format!("未能从系统钥匙串读取保险箱密钥（{e}）"))?;
    hex_decode(&stored).ok_or_else(|| "钥匙串中的保险箱密钥格式损坏".to_string())
}

/// 把 32B key 写入系统钥匙串（hex 字符串形式）。
fn keyring_set(key: &[u8]) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("系统钥匙串不可用（{e}）"))?;
    entry
        .set_password(&hex_encode(key))
        .map_err(|e| format!("写入系统钥匙串失败（{e}）"))
}

/// 删除钥匙串中的 key（setup 落盘失败时回滚，避免孤儿 key）。
fn keyring_delete() {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.delete_credential();
    }
}

/// add/update 的字段集合（与 core `VaultEntryInput` 同形；password 空串=不改）。
#[derive(Deserialize)]
struct VaultEntryFields {
    site: String,
    account: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    icon: String,
}

impl From<VaultEntryFields> for qibai_core::vault::VaultEntryInput {
    fn from(f: VaultEntryFields) -> Self {
        qibai_core::vault::VaultEntryInput {
            site: f.site,
            account: f.account,
            password: f.password,
            note: f.note,
            icon: f.icon,
        }
    }
}

/// 保险箱状态：keyring_available 由本层探测后传入（契约 §4 VaultStatus）。
#[tauri::command]
async fn vault_status(state: tauri::State<'_, AppState>) -> Result<qibai_core::vault::VaultStatus, String> {
    let available = keyring_available();
    Ok(state.engine.lock().await.vault_status(available))
}

/// 主密码模式初始化（仅 unset；core 内部生成盐/verifier/空库落盘）。
#[tauri::command(rename_all = "snake_case")]
async fn vault_setup(
    state: tauri::State<'_, AppState>,
    master_password: String,
) -> Result<String, String> {
    state
        .engine
        .lock().await
        .vault_setup(&master_password)
        .map_err(|e| e.to_string())?;
    Ok("ok".to_string())
}

/// keyring 模式初始化：本层生成 32B 随机 key → 写系统钥匙串 → 交 core 落盘。
/// 钥匙串不可用 / 写盘失败回滚钥匙串条目，结构化提示前端走主密码模式。
#[tauri::command]
async fn vault_setup_keyring(state: tauri::State<'_, AppState>) -> Result<String, String> {
    if !keyring_available() {
        return Err(
            "VaultKeyringUnavailable: 系统钥匙串不可用，请改用主密码模式初始化".to_string()
        );
    }
    let key = random_master_key()?;
    keyring_set(&key)?;
    let result = state
        .engine
        .lock().await
        .vault_setup_keyring(&key)
        .map_err(|e| e.to_string());
    if result.is_err() {
        // 回滚：core 未落盘则钥匙串不留孤儿 key
        keyring_delete();
    }
    result?;
    Ok("ok".to_string())
}

/// 主密码解锁（防暴力计数/冷却在 core 内）。
#[tauri::command(rename_all = "snake_case")]
async fn vault_unlock(
    state: tauri::State<'_, AppState>,
    master_password: String,
) -> Result<String, String> {
    state
        .engine
        .lock().await
        .vault_unlock(&master_password)
        .map_err(|e| e.to_string())?;
    Ok("ok".to_string())
}

/// keyring 模式系统解锁：本层取钥匙串 key → core 校验（不计数失败）。
#[tauri::command]
async fn vault_unlock_system(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let key = keyring_get()?;
    state
        .engine
        .lock().await
        .vault_unlock_system(&key)
        .map_err(|e| e.to_string())?;
    Ok("ok".to_string())
}

/// 立即锁定（core 内 zeroize 内存 key）。
#[tauri::command]
async fn vault_lock(state: tauri::State<'_, AppState>) -> Result<String, String> {
    state
        .engine
        .lock().await
        .vault_lock()
        .map_err(|e| e.to_string())?;
    Ok("ok".to_string())
}

/// 条目列表（仅 unlocked；core 内自动 touch）。
#[tauri::command]
async fn vault_list(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<qibai_core::vault::VaultEntry>, String> {
    state
        .engine
        .lock().await
        .vault_list()
        .map_err(|e| e.to_string())
}

/// 本地过滤检索（site/account/note，大小写不敏感、中文子串；仅 unlocked）。
#[tauri::command]
async fn vault_search(
    state: tauri::State<'_, AppState>,
    q: String,
) -> Result<Vec<qibai_core::vault::VaultEntry>, String> {
    state
        .engine
        .lock().await
        .vault_search(&q)
        .map_err(|e| e.to_string())
}

/// 新增条目 → `{ id }`。
#[tauri::command]
async fn vault_add(
    state: tauri::State<'_, AppState>,
    site: String,
    account: String,
    password: String,
    note: String,
    icon: String,
) -> Result<serde_json::Value, String> {
    let fields = VaultEntryFields { site, account, password, note, icon };
    let id = state
        .engine
        .lock().await
        .vault_add(fields.into())
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "id": id }))
}

/// 更新条目（password 空串表示不改）→ `"ok"`。
#[tauri::command]
async fn vault_update(
    state: tauri::State<'_, AppState>,
    id: String,
    site: String,
    account: String,
    password: String,
    note: String,
    icon: String,
) -> Result<String, String> {
    let fields = VaultEntryFields { site, account, password, note, icon };
    state
        .engine
        .lock().await
        .vault_update(&id, fields.into())
        .map_err(|e| e.to_string())?;
    Ok("ok".to_string())
}

#[tauri::command]
async fn vault_delete(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    state
        .engine
        .lock().await
        .vault_delete(&id)
        .map_err(|e| e.to_string())?;
    Ok("ok".to_string())
}

/// 复制密码到系统剪贴板并调度清除。
/// core 返回明文+clear_after_secs；本层写剪贴板后 spawn 计时任务清空。
/// 返回载荷严格不含明文（契约 §4/§7）。
#[tauri::command]
async fn vault_copy(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let hold = state
        .engine
        .lock().await
        .vault_copy(&id)
        .map_err(|e| e.to_string())?;

    // 明文瞬时写系统剪贴板；不进日志、不进返回值。
    app.clipboard()
        .write_text(hold.plaintext)
        .map_err(|e| format!("写入系统剪贴板失败（{e}）"))?;

    // spawn 清除任务：clear_after_secs 后清空剪贴板。
    // 注意：若期间用户又复制了别的内容，本任务会一并清空——契约接受的折中，
    // 真机验证「写入→等待→已清空」行为。
    let clear_app = app.clone();
    let clear_secs = hold.clear_after_secs;
    tauri::async_runtime::spawn(async move {
        sleep(std::time::Duration::from_secs(clear_secs)).await;
        let _ = clear_app.clipboard().write_text("");
    });

    Ok(serde_json::json!({
        "copied": true,
        "clear_after_secs": hold.clear_after_secs,
    }))
}

/// 导出备份：core 生成 .qbkv 信封 → 本层写 `<data_dir>/vault-backup-<ts>.qbkv` → `{ path }`。
#[tauri::command]
async fn vault_export(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let payload = state
        .engine
        .lock().await
        .vault_export()
        .map_err(|e| e.to_string())?;
    let dir = qibai_core::settings::data_dir().map_err(|e| e.to_string())?;
    let path = dir.join(format!("vault-backup-{}.qbkv", now_unix_secs()));
    std::fs::write(&path, payload).map_err(|e| format!("备份文件写盘失败：{e}"))?;
    Ok(serde_json::json!({ "path": path.to_string_lossy().to_string() }))
}

/// 导入备份（纯转发；本地状态机分支/重加密合并在 core 内，成功后统一回 locked）。
#[tauri::command(rename_all = "snake_case")]
async fn vault_import_payload(
    state: tauri::State<'_, AppState>,
    payload: String,
    master_password: String,
) -> Result<String, String> {
    state
        .engine
        .lock().await
        .vault_import_payload(&payload, &master_password)
        .map_err(|e| e.to_string())?;
    Ok("ok".to_string())
}

// ---------- 提醒后台 tick ----------

/// 发一条系统通知（跨三平台，经 tauri-plugin-notification）。
fn send_system_notification(app: &tauri::AppHandle, ev: &ReminderEvent) {
    use tauri_plugin_notification::NotificationExt;
    let kind = match ev.kind {
        qibai_core::ReminderKind::Sit => "久坐提醒",
        qibai_core::ReminderKind::Water => "喝水提醒",
        qibai_core::ReminderKind::Eye => "护眼提醒",
    };
    let _ = app
        .notification()
        .builder()
        .title(format!("元气小小 · {kind}"))
        .body(&ev.message)
        .show();
}

/// 每 15 秒检查一次提醒；到期事件既经 EventSink 推给前端，也发系统通知。
/// 久坐（Sit）事件额外透传给守护调度器：触发成功且需全屏演出时，
/// 复用 guard_trigger 的窗口创建 + `qibai://guard-start` 广播逻辑。
fn spawn_reminder_tick(app: tauri::AppHandle, engine: Arc<Mutex<Engine>>) {
    tauri::async_runtime::spawn(async move {
        let mut tick = interval(std::time::Duration::from_secs(15));
        loop {
            tick.tick().await;
            let (guard_starts, speaks, vault_just_locked): (
            Vec<qibai_core::guard::GuardStart>,
            Vec<TtsSpeakRequest>,
            bool,
        ) = {
                let mut engine = engine.lock().await;
                let events = engine.tick_reminders();
                for ev in &events {
                    send_system_notification(&app, ev);
                }
                let now = now_unix_secs();
                let mut starts = Vec::new();
                for ev in &events {
                    if ev.kind == qibai_core::ReminderKind::Sit {
                        if let Some(start) = engine.guard_on_sit(now) {
                            starts.push(start);
                        }
                    }
                }
                // 迭代②：提醒语音播报（Reminder scope / 总开关 / 文本非空门控在 tts_decide 内）
                // 迭代⑤：提醒无专属角色归属 → persona_id=None（core 用主角色音色兜底）。
                let mut speaks = Vec::new();
                for ev in &events {
                    if let Some(req) = engine.tts_decide(tts::TtsKind::Reminder, &ev.message, None) {
                        speaks.push(req);
                    }
                }
                // 迭代③：保险箱闲置超时检查（core 内部按 settings.vault.idle_lock_min 判定；
                // 返回 true 表示本次 tick 刚自动锁定）。
                let vault_just_locked = engine.vault_tick();
                (starts, speaks, vault_just_locked)
            };
            // 刚发生闲置自动锁定 → 通知前端刷新（契约 §4 可选事件 qibai://vault-locked）。
            if vault_just_locked {
                let _ = app.emit(
                    "qibai://vault-locked",
                    serde_json::json!({ "reason": "idle" }),
                );
            }
            for req in speaks {
                // P0-B 调优：提醒口播过投递窗口——窗口不安全（用户说话/口播挂起中）
                // 本轮跳过，防误 barge-in 后后台结果直接抢播（自说自话观感）。
                if qibai_core::tts::delivery_window_now().is_safe() {
                    perform_speech(&app, &engine, req).await;
                } else {
                    eprintln!("[qibai] reminder 口播: 投递窗口不安全→ 本轮跳过");
                }
            }
            for start in guard_starts {
                if start.perform_fullscreen {
                    start_guard_performance(&app, &engine, start).await;
                }
            }
        }
    });
}

// ---------- 托盘（契约 §2.3：打开控制台 / 显示桌宠 / 隐藏桌宠 / 退出） ----------

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::{
        menu::{Menu, MenuItem},
        tray::TrayIconBuilder,
    };

    let console = MenuItem::with_id(app, "console", "打开控制台", true, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", "显示桌宠", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "隐藏桌宠", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&console, &show, &hide, &quit])?;

    TrayIconBuilder::with_id("qibai-tray")
        .tooltip("元气小小 Qibai")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "console" => {
                let _ = ensure_console(app);
            }
            "show" => {
                for (_, w) in app.webview_windows() {
                    if w.label().starts_with("pet-") {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
            }
            "hide" => {
                for (_, w) in app.webview_windows() {
                    if w.label().starts_with("pet-") {
                        let _ = w.hide();
                    }
                }
            }
            "quit" => {
                // 问题7优雅 shutdown：退出进程前 best-effort 优雅断开 netsocial WS
                // （发 WsCmd::Shutdown，ms 级不阻塞），避免服务端等心跳超时才发现客户端没了。
                // mic/cpal 流与 reminder tick 由运行时退出回收（真机验证项）。
                if let Some(state) = app.try_state::<AppState>() {
                    let eng = state.engine.clone();
                    tauri::async_runtime::block_on(async {
                        let mut g = eng.lock().await;
                        let _ = g.netsocial_disconnect();
                    });
                }
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}

// ---------- 迭代⑥：自动更新 ----------

/// 自动更新当前处于未启用态：无真实发布源（endpoint/pubkey 占位）。
/// tauri-plugin-updater 已移除——其未注册时调用 app.updater() 会直接 panic
/// （state() called before manage()），导致 tokio worker 崩溃、窗口闪退。
/// 待接入真实发布源（GitHub Releases / 自建 OTA）时按 docs/ 部署文档启用。

/// 手动"检查更新"返回的结构化状态（控制台设置页调用）。
#[derive(serde::Serialize)]
struct CheckUpdateStatus {
    /// available=有可用更新 / latest=已是最新 / disabled=未启用 / error=检查失败
    status: String,
    /// 当前运行版本
    current_version: String,
    /// 远端最新版本（available 时给出）
    latest_version: Option<String>,
    /// 失败原因（error 时给出）
    error: Option<String>,
}

// ---- 全量契约补齐（v14）：专家/插件/网络社交命令注册包装 ----
#[tauri::command]
async fn expert_list(state: tauri::State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    Ok(state.engine.lock().await.expert_list())
}

#[tauri::command]
async fn plugin_list(state: tauri::State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    Ok(state.engine.lock().await.plugin_list())
}

// ---- 连接器（MCP/HTTP 自定义连接器） ----

/// 新增自定义连接器（MCP stdio 或 HTTP 端点）：物化 → 注册 → 自动授权 → 启用 → 落盘。
#[tauri::command]
async fn connector_add(
    state: tauri::State<'_, AppState>,
    id: String,
    name: String,
    kind: String,
    command: String,
    args: Vec<String>,
    // 前端按需只填一侧（MCP 只给 command，HTTP 只给 url），url/headers 必须可缺省，
    // 否则缺 key 时 Tauri 直接判「参数不合法」，添加连接器按钮必失败。
    url: Option<String>,
    headers: Option<serde_json::Value>,
    description: String,
    enabled: bool,
) -> Result<(), String> {
    let headers: std::collections::BTreeMap<String, String> = match headers {
        Some(v) if !v.is_null() => {
            serde_json::from_value(v).map_err(|e| format!("headers 解析失败: {e}"))?
        }
        _ => std::collections::BTreeMap::new(),
    };
    let cfg = qibai_core::connectors::ConnectorConfig {
        id,
        name,
        kind: match kind.as_str() {
            "mcp" => qibai_core::connectors::ConnectorKind::Mcp,
            "http" => qibai_core::connectors::ConnectorKind::Http,
            other => return Err(format!("不支持的连接器类型 {other}（mcp/http）")),
        },
        command,
        args,
        url: url.unwrap_or_default(),
        headers,
        description,
        enabled,
    };
    state
        .engine
        .lock()
        .await
        .connector_add(cfg)
        .map_err(|e| e.to_string())
}

/// 移除自定义连接器（删除物化目录 + 摘除注册 + 清 settings）。
#[tauri::command]
async fn connector_remove(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    state
        .engine
        .lock()
        .await
        .connector_remove(&id)
        .map_err(|e| e.to_string())
}

/// 连接器清单（type=connector 插件）。
#[tauri::command]
async fn connector_list(state: tauri::State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    Ok(state.engine.lock().await.connector_list())
}

// ---- 技能包（.qbpk 打包/导入/导出） ----

/// 导入安装技能包（zip）：安全解压 → manifest 校验 → 复制进 plugins/ → 注册。
#[tauri::command(rename_all = "snake_case")]
async fn skill_pack_install(state: tauri::State<'_, AppState>, zip_path: String) -> Result<(), String> {
    state
        .engine
        .lock()
        .await
        .skill_pack_install(zip_path)
        .map_err(|e| e.to_string())
}

/// 导入安装技能包（字节形态）：前端把 .qbpk 内容读成 base64 传来。
/// WebView 的 `<input type=file>` 给不出可信路径（早先前端传 `demo://文件名` 必然失败），
/// 所以这里解码 → 交引擎落临时文件 → 复用同一条安全校验与安装链。
#[tauri::command(rename_all = "snake_case")]
async fn skill_pack_install_bytes(
    state: tauri::State<'_, AppState>,
    file_name: String,
    data_b64: String,
) -> Result<serde_json::Value, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64.trim())
        .map_err(|e| format!("技能包内容解码失败（多半是读取中断或文件损坏）: {e}"))?;
    let mut engine = state.engine.lock().await;
    engine
        .skill_pack_install_bytes(file_name, &bytes)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"ok": true, "bytes": bytes.len()}))
}

/// 技能安装 stage（字节形态）：同 skill_pack 的思路，前端读文件内容后在这里落临时文件，
/// 再复用 `.md` / `.zip` 两条既有暂存链，避免依赖 WebView 不可靠的 `f.path`。
#[tauri::command(rename_all = "snake_case")]
async fn skill_install_stage_bytes(
    state: tauri::State<'_, AppState>,
    file_name: String,
    data_b64: String,
) -> Result<serde_json::Value, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64.trim())
        .map_err(|e| format!("文件内容解码失败（多半是读取中断或文件损坏）: {e}"))?;
    state
        .engine
        .lock()
        .await
        .skill_install_stage_bytes(&file_name, &bytes)
}

/// 导出技能包到 <数据目录>/exports/，返回落盘路径。
#[tauri::command(rename_all = "snake_case")]
async fn skill_pack_export_default(
    state: tauri::State<'_, AppState>,
    plugin_id: String,
) -> Result<serde_json::Value, String> {
    let path = state
        .engine
        .lock()
        .await
        .skill_pack_export_default(plugin_id)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"ok": true, "path": path}))
}

/// 导出技能包（zip）：从 plugins/<id> 打包。
#[tauri::command(rename_all = "snake_case")]
async fn skill_pack_export(
    state: tauri::State<'_, AppState>,
    plugin_id: String,
    zip_path: String,
) -> Result<(), String> {
    state
        .engine
        .lock()
        .await
        .skill_pack_export(plugin_id, zip_path)
        .map_err(|e| e.to_string())
}

/// 可导出技能包清单。
#[tauri::command]
async fn skill_pack_list(state: tauri::State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    Ok(state.engine.lock().await.skill_pack_list())
}

#[tauri::command]
async fn netsocial_events(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_events().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_poll(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    Ok(state.engine.lock().await.netsocial_poll())
}

/// 问题7补充修复：半接线命令补注册——删除加密缓存两文件并重置联网社交身份。
#[tauri::command]
async fn netsocial_cache_clear(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_cache_clear().map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_status(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    Ok(state.engine.lock().await.netsocial_status())
}

#[tauri::command]
async fn netsocial_topics(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_topics().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn expert_set_active(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    state.engine.lock().await.expert_set_active(id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn plugin_grant_permission(state: tauri::State<'_, AppState>, id: String, permission: String, approve: bool) -> Result<(), String> {
    state.engine.lock().await.plugin_grant_permission(id, permission, approve).map_err(|e| e.to_string())
}

#[tauri::command]
async fn plugin_install(state: tauri::State<'_, AppState>, dir: String) -> Result<(), String> {
    state.engine.lock().await.plugin_install(dir).map_err(|e| e.to_string())
}

/// v16：窗口拖动——按增量移动窗口（不依赖 WebKitGTK 的 drag-region 属性）。
/// 前端错误转发（诊断）：前端任何 invoke 失败会调此命令把错误打进后端日志。
#[tauri::command]
async fn debug_log(msg: String) {
    eprintln!("[qibai][frontend] {msg}");
}

#[tauri::command]
async fn pet_move_by(window: tauri::Window, dx: i32, dy: i32) -> Result<(), String> {
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    // 问题6/前端P1：多屏场景下副屏坐标为负（如主屏左侧显示器 x=-1920），
    // 此前 `.max(0)` 会把合法负坐标钳到 0，把窗口错误拽到主屏原点。直接用物理坐标。
    let new_x = pos.x as i32 + dx;
    let new_y = pos.y as i32 + dy;
    window
        .set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
            new_x, new_y,
        )))
        .map_err(|e| e.to_string())
}

/// 问题6：桌宠窗口靠近屏幕边缘时自动吸附。
/// 契约：invoke('pet_snap_to_edge') 无前端参数；threshold=14px 在 Rust 侧常量。
/// 按当前 monitor 的 work_area() 四边吸附（多屏按窗口所在显示器工作区，不跨屏拽）。
/// 返回 {snapped:bool, edge:'left'|'right'|'top'|'bottom'|'none', x:number, y:number}。
#[tauri::command]
async fn pet_snap_to_edge(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    // 设置开关：ui.snap_to_edge=false 时不执行吸附位移，但**仍要执行拖动结束清理**。
    // v26 修复：此前开关关闭时 early-return，把 webview 重排清残影也一并跳过——
    // 前端在 pointerup 固定调本命令，清理被开关短路 = 关闭吸附的用户松手后残影无人清。
    let snap_enabled = state.engine.lock().await.settings.ui.snap_to_edge;
    const THRESHOLD: i32 = 14; // px：窗口边距 ≤ 此值即吸附
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let wx = pos.x;
    let wy = pos.y;
    let ww = size.width as i32;
    let wh = size.height as i32;
    // 窗口所在显示器的工作区（多屏按此屏，不跨屏）。
    let monitor = window.current_monitor().map_err(|e| e.to_string())?;
    let Some(mon) = monitor else {
        return Ok(serde_json::json!({"snapped": false, "edge": "none", "x": wx, "y": wy}));
    };
    let wa = mon.work_area();
    let wa_x = wa.position.x;
    let wa_y = wa.position.y;
    let wa_r = wa.position.x + wa.size.width as i32;
    let wa_b = wa.position.y + wa.size.height as i32;
    // 窗口四边到工作区四边的距离（负=已越界，吸附会拉回）。
    let dists = [
        ("left", wx - wa_x),
        ("top", wy - wa_y),
        ("right", wa_r - (wx + ww)),
        ("bottom", wa_b - (wy + wh)),
    ];
    let mut edge = "none";
    let mut tx = wx;
    let mut ty = wy;
    if let Some((e, d)) = dists.iter().min_by_key(|(_, d)| d.abs()) {
        if d.abs() <= THRESHOLD {
            edge = e;
            match *e {
                "left" => tx = wa_x,
                "right" => tx = wa_r - ww,
                "top" => ty = wa_y,
                "bottom" => ty = wa_b - wh,
                _ => {}
            }
        }
    }
    // 吸附位移仅在开关开启且命中边缘时执行；无论是否命中都走末尾的 webview 重排清理。
    let snapped = snap_enabled && edge != "none";
    if snapped {
        window
            .set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(tx, ty)))
            .map_err(|e| e.to_string())?;
    }
    // 重影深查：拖动结束/snap 瞬移后旧位置像素残留（无合成器 X11），主动触发一次
    // webview JS reflow 清残影（与 pet_repaint 同一手段）。开关关闭时也执行——
    // 本命令是前端 pointerup 的固定收尾调用，清理不能被吸附开关短路。
    let _ = window.eval(PET_REFLOW_JS);
    Ok(serde_json::json!({"snapped": snapped, "edge": edge, "x": tx, "y": ty}))
}

/// 诊断辅助：PATH 中是否存在该播放器/引擎。
fn tts_which_binary(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|d| d.join(name).is_file()))
        .unwrap_or(false)
}

/// v18.7：系统级窗口拖动（GTK begin_move_drag）。比逐帧 set_position 跟手，
/// 且在无合成器 X11 下避免透明窗口逐帧移动产生的重影/残框。
///
/// v44：返回 `{moved, x, y}`。GTK begin_move_drag 在 X11 下以 nested main loop
/// 阻塞到用户松手才返回；此时比对拖拽起止窗口坐标得出 moved，供前端确定性区分
/// 「真拖过（压制 click，防误触发摸头）」与「纯点按（放行 click）」，不再依赖
/// webview 是否能收到 pointerup。
#[tauri::command]
fn pet_start_drag(window: tauri::Window) -> Result<serde_json::Value, String> {
    eprintln!("[qibai] drag: enter pet_start_drag (IPC 已到达 Rust)");
    // v18.8-fix：同步命令（非 async）——GTK begin_move_drag 对事件时序敏感，
    // async 命令的 IPC 调度延迟易使其失败，回退逐帧移动会在 X11 无合成器下产生橙色残框。
    let start = window
        .outer_position()
        .unwrap_or(tauri::PhysicalPosition::new(0, 0));
    match window.start_dragging() {
        Ok(()) => {
            let end = window.outer_position().unwrap_or(start);
            let moved = (end.x - start.x).abs() + (end.y - start.y).abs() > 3;
            eprintln!(
                "[qibai] drag: system move ended moved={moved} start=({},{}) end=({},{})",
                start.x, start.y, end.x, end.y
            );
            Ok(serde_json::json!({ "moved": moved, "x": end.x, "y": end.y }))
        }
        Err(e) => {
            eprintln!("[qibai] drag: system move FAILED: {e} → 回退逐帧移动");
            Err(e.to_string())
        }
    }
}

/// v44：置顶重申。前端启动/回焦/每 6 秒调用一次，抵消 WM 在 unminimize、
/// resize、切形态后丢弃 _NET_WM_STATE_ABOVE 的问题；只改 keep-above，不抢焦点。
#[tauri::command]
fn pet_keep_on_top(window: tauri::Window) -> Result<(), String> {
    window.set_always_on_top(true).map_err(|e| e.to_string())
}

/// v18.8-fix4：强制窗口重绘（尺寸 ±1px 抖动触发 X/GTK 全量重绘），
/// 用于清除透明窗口在重绘缓存下的残影（拖动残框、气泡重影、切角色重影）。
///
/// v20新P0（透明窗重影）：新增 `force: Option<bool>` 参数——
/// - 默认/缺省 = true：执行尺寸 ±1px 抖动强制重绘（保持原行为，向后兼容）。
/// - force=false：跳过重绘——供前端在**拖动过程中高频节流**调用（拖动中再抖动尺寸会跳变）；
/// - force=true：**拖动结束 / 重影出现时**强制抖动重绘（清残框）。
/// 前端官契约：`invoke('pet_repaint', { force: true })` 拖动结束调；拖动中可不调或 force:false。
/// v20重影深查：tao 官方 release note 明确「Linux 上 size 方法是异步的，连续两次
/// set_size 不可靠」——set_size(w-1)→set_size(w) 在 Linux 会被 WM 合并，中间 w-1
/// 态从不提交，不触发 configure/resize，webview 不重绘。故以 webview JS 强制 reflow
/// 为主手段（绕过 WM），尺寸抖动仅作 Windows/macOS 兜底。
const PET_REFLOW_JS: &str = "(()=>{try{const b=document.body;const d=b.style.display;b.style.display='none';void b.offsetHeight;b.style.display=d;}catch(e){}})()";

/// v45.6：强制重绘窗口——只走 webview 内部 reflow（eval PET_REFLOW_JS）。
/// 所有原生 X/GTK 表面操作已整条删除（真机证据：X RENDER BadDrawable → SIGTRAP，
/// 见 docs/v45.6 说明）。原则：只要窗口有 hide/show（切角色、气泡收尾），
/// 就绝不并发任何原生表面操作——hide 会销毁 drawable，正是 BadDrawable 的温床。
fn repaint_window_forced(window: &tauri::WebviewWindow, _full: bool) {
    let _ = window.eval(PET_REFLOW_JS);
}

#[tauri::command]
fn pet_repaint(
    window: tauri::WebviewWindow,
    force: Option<bool>,
    full: Option<bool>,
) -> Result<(), String> {
    if !force.unwrap_or(true) {
        return Ok(()); // 前端显式跳过重绘（拖动中节流）
    }
    repaint_window_forced(&window, full.unwrap_or(false));
    Ok(())
}

/// v45.6：本命令保留注册（前端还在 invoke），但实现为空——前端 canvas 自行重绘。
#[tauri::command]
async fn pet_surface_rebuild(_window: tauri::WebviewWindow) -> Result<(), String> {
    eprintln!("[qibai] surface-op disabled (native X surface ops removed in v45.6)");
    Ok(())
}

/// v45.6：本命令保留注册（前端还在 invoke），但实现为空——前端 canvas 自行重绘。
#[tauri::command]
async fn pet_invalidate_rect(
    _window: tauri::WebviewWindow,
    _x: f64, _y: f64, _w: f64, _h: f64,
) -> Result<(), String> {
    eprintln!("[qibai] surface-op disabled (native X surface ops removed in v45.6)");
    Ok(())
}

/// v20.3：控制台窗强制重绘（reflow 清不掉无合成器 X11 的窗口表面旧像素，
/// 「切 tab 后旧内容/卡片残留不消失」的根）。与 pet_repaint 同一手段。
#[tauri::command]
fn console_repaint(window: tauri::WebviewWindow) -> Result<(), String> {
    repaint_window_forced(&window, true);
    Ok(())
}

#[tauri::command]
async fn skill_list(state: tauri::State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    Ok(state.engine.lock().await.skill_list())
}

/// v18.1：技能输出面板化。解析 @qibai-panel 协议块并展示：
/// bubble → 桌宠窗气泡卡片；window → 独立面板窗。返回去协议后的朗读文本。
fn show_panel_payload(
    app: &tauri::AppHandle,
    payload: &qibai_core::panel::PanelPayload,
) -> tauri::Result<()> {
    if payload.mode == "bubble" {
        let _ = app.emit(
            "qibai://ui-panel",
            serde_json::json!({ "title": payload.title, "html": payload.html }),
        );
        return Ok(());
    }
    let _win = ensure_panel_window(app, 720.0, 560.0)?;
    let _ = app.emit(
        "qibai://panel-content",
        serde_json::json!({ "title": payload.title, "html": payload.html }),
    );
    Ok(())
}

#[tauri::command]
async fn skill_invoke(app: tauri::AppHandle, state: tauri::State<'_, AppState>, id: String, function: String, args: String) -> Result<String, String> {
    let out = state
        .engine
        .lock()
        .await
        .skill_invoke(&id, &function, &args)
        .map_err(|e| e.to_string())?;
    let (payload, speak) = qibai_core::panel::parse_panel_annotation(&out);
    if let Some(p) = payload {
        show_panel_payload(&app, &p).map_err(|e| e.to_string())?;
    }
    Ok(speak)
}

#[tauri::command]
async fn scan_plugin_library(state: tauri::State<'_, AppState>, dir: String) -> Result<usize, String> {
    Ok(state.engine.lock().await.scan_plugin_dir(std::path::Path::new(&dir)))
}

#[tauri::command]
async fn plugin_set_enabled(state: tauri::State<'_, AppState>, id: String, enabled: bool) -> Result<(), String> {
    state.engine.lock().await.plugin_set_enabled(id, enabled).map_err(|e| e.to_string())
}

#[tauri::command]
async fn plugin_uninstall(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    state.engine.lock().await.plugin_uninstall(id).map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_block(state: tauri::State<'_, AppState>, id: String) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_block(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_city_souls(state: tauri::State<'_, AppState>, city: String) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_city_souls(&city).await.map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "snake_case")]
async fn netsocial_connect(state: tauri::State<'_, AppState>, server_url: Option<String>) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_connect(server_url).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_disconnect(state: tauri::State<'_, AppState>, ) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_disconnect().map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_history(state: tauri::State<'_, AppState>, id: Option<String>) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_history(id.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_like(state: tauri::State<'_, AppState>, id: String) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_like(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_match(state: tauri::State<'_, AppState>, limit: Option<u32>) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_match(limit).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_register(state: tauri::State<'_, AppState>, input: serde_json::Value) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_register(&input).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_report(state: tauri::State<'_, AppState>, id: String, reason: String) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_report(&id, &reason).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_roam(state: tauri::State<'_, AppState>, params: serde_json::Value) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_roam(&params).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_send(state: tauri::State<'_, AppState>, id: String, text: String) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_send(&id, &text).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn netsocial_set_profile(state: tauri::State<'_, AppState>, input: serde_json::Value) -> Result<serde_json::Value, String> {
    state.engine.lock().await.netsocial_set_profile(&input).await.map_err(|e| e.to_string())
}

// ---- 免提聆听（v14）：语音唤醒开关 + 状态 ----
#[tauri::command]
async fn set_wake_enabled(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<serde_json::Value, String> {
    {
        let mut e = state.engine.lock().await;
        e.settings.voice.wake_enabled = enabled;
        e.settings.save().map_err(|err| format!("{err:?}"))?;
    }
    if enabled {
        match listener::start_listener(app.clone(), state.engine.clone(), state.listen.clone()) {
            Ok(()) => {
                eprintln!("[qibai] 免提聆听已启动（唤醒词：{}）",
                    state.engine.lock().await.settings.voice.wake_word);
                Ok(serde_json::json!({"ok": true, "state": "wake"}))
            }
            Err(e) => {
                eprintln!("[qibai] 免提聆听启动失败: {e}");
                Ok(serde_json::json!({"ok": false, "reason": e}))
            }
        }
    } else {
        listener::stop_listener(&state.listen);
        Ok(serde_json::json!({"ok": true, "state": "off"}))
    }
}

#[tauri::command]
async fn get_listen_state(state: tauri::State<'_, AppState>) -> Result<serde_json::Value, String> {
    let (enabled, wake_word, stt_enabled) = {
        let e = state.engine.lock().await;
        (
            e.settings.voice.wake_enabled,
            e.settings.voice.wake_word.clone(),
            e.settings.voice.stt_enabled,
        )
    };
    let st = state.listen.state.load(std::sync::atomic::Ordering::SeqCst);
    let state_name = match st {
        listener::LISTEN_ACTIVE => "active",
        listener::LISTEN_WAKE => "wake",
        _ => "off",
    };
    Ok(serde_json::json!({
        "enabled": enabled,
        "state": state_name,
        "wake_word": wake_word,
        "stt_enabled": stt_enabled,
    }))
}

#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> CheckUpdateStatus {
    let current_version = app.package_info().version.to_string();
    CheckUpdateStatus {
        status: "disabled".into(),
        current_version,
        latest_version: None,
        error: Some("自动更新未启用（未配置发布源）".into()),
    }
}

/// v20.2/v20.3：单实例锁（Linux）。启动最前调用：
/// 若 /tmp/qibai.pid 记录的旧实例仍存活，先终止再继续。根治：
/// ① start_qibai.sh 的 stop 只杀服务端、start 又因客户端在跑而跳过重启
///    → 覆盖+编译后的新二进制从不生效（“怎么改都没解决”的根）；
/// ② 多个实例的 pet 窗叠在同一贴边位置 = 「两个/多个完整立绘重叠」。
/// v20.3 加固：杀旧实例前校验 /proc/<pid>/cmdline 确含 "qibai-tauri"，
/// 防止误杀占用该 pid 文件的其他程序；等待上限缩短到 2s，防启动阻塞/竞态。
fn enforce_single_instance() {
    #[cfg(target_os = "linux")]
    {
        if let Ok(pid_str) = std::fs::read_to_string("/tmp/qibai.pid") {
            if let Ok(old_pid) = pid_str.trim().parse::<i32>() {
                let me = std::process::id() as i32;
                if old_pid > 0 && old_pid != me {
                    let exe_path = format!("/proc/{}/exe", old_pid);
                    let proc_exe = std::path::Path::new(&exe_path);
                    let alive = proc_exe.exists();
                    let is_qibai = if alive {
                        std::fs::read_to_string(format!("/proc/{}/cmdline", old_pid))
                            .map(|c| c.contains("qibai-tauri"))
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    if is_qibai {
                        eprintln!(
                            "[qibai] 单实例锁：终止旧实例 pid={}（确保新代码生效）",
                            old_pid
                        );
                        let _ = std::process::Command::new("kill")
                            .arg(old_pid.to_string())
                            .status();
                        for _ in 0..20 {
                            if !proc_exe.exists() {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                    }
                }
            }
        }
        // 登记当前实例
        let _ = std::fs::write("/tmp/qibai.pid", std::process::id().to_string());
    }
}

/// v45.3：进程级 panic hook。
///
/// 真机「按住麦克风说话时进程直接退出」最可能的原因：cpal 音频回调线程里
/// `Mutex::lock().unwrap()` 遇到 poisoned lock 直接 panic，panic 在 ALSA 实时回调
/// 内 unpin 到 C 层 → SIGABRT/段错误，整个进程消失。
///
/// 这个 hook 做两件事：
/// 1. 把 panic 信息追加写到 `~/.local/share/qibai/panic.log`（真机取证）；
/// 2. 尝试 emit 一个 `qibai://panic-fatal` 事件（WebView 已起来的话），
///    让前端 toast 提示「音频子系统崩溃，已降级为纯文本」，而不是悄无声息退出。
///
/// 注意：hook 里不能再 panic；所有写盘/emit 都用 `let _ = ...` 吞掉错误。
static PANIC_APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let log_dir = dirs_user_data();
        let log_path = log_dir.join("panic.log");
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        // v45.4：强制抓 backtrace（前 20 行），真机取证才拿得到栈。
        // RUST_BACKTRACE=1 环境下 Backtrace::force_capture() 会输出符号化栈。
        let bt = std::backtrace::Backtrace::force_capture();
        let bt_str = format!("{bt}");
        let bt_head: String = bt_str.lines().take(20).collect::<Vec<_>>().join("\n");
        let payload = format!(
            "[{}] thread={:?} panic: {}\n--- backtrace (head 20) ---\n{}\n--- end ---\n",
            ts,
            std::thread::current().name().unwrap_or("<unnamed>"),
            info,
            bt_head,
        );
        let _ = std::fs::create_dir_all(&log_dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
            use std::io::Write;
            let _ = f.write_all(payload.as_bytes());
        }
        eprintln!("[qibai PANIC] {}", payload.trim());
        if let Some(h) = PANIC_APP.get() {
            let _ = h.emit(
                "qibai://panic-fatal",
                serde_json::json!({"msg": info.to_string()}),
            );
        }
        default(info);
    }));
}

/// 取用户数据目录：~/.local/share/qibai（Linux）、~/Library/Application Support/qibai（macOS）。
fn dirs_user_data() -> std::path::PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let p = std::path::Path::new(&home);
        if cfg!(target_os = "macos") {
            p.join("Library/Application Support/qibai")
        } else {
            p.join(".local/share/qibai")
        }
    } else {
        std::path::PathBuf::from(".")
    }
}

fn main() {
    // v45.3：进程级 panic hook——把 panic 写到 ~/.local/share/qibai/panic.log，
    // 并 emit 前端事件 qibai://panic-fatal 让 UI 弹「音频/子系统崩溃已降级」提示，
    // 而不是进程直接消失（真机按住麦克风闪退的取证底座）。
    install_panic_hook();
    eprintln!("[qibai] boot 01: hook installed");
    // v45.4 safe-mode 开关启动横幅（一次打完，真机日志一眼可见）。
    eprintln!(
        "[qibai] safe-mode: OPAQUE={} AUDIO={} COMPOSITOR_PROBE={}",
        if safe_opaque() { 1 } else { 0 },
        if safe_no_audio() { 0 } else { 1 },
        if safe_no_compositor_probe() { 0 } else { 1 },
    );
    enforce_single_instance();
    // 启动横幅：日志首行打印构建标识，便于真机排错时确认运行的是哪个版本。
    eprintln!("[qibai] 元气小小 v13 build=updater-removed tolerant-startup");
    // Linux 渲染路径（默认软渲染优先）：
    // 真机实测：本地桌面 + NVIDIA 专有驱动下，WebKit 的窗口级 EGL 仍可能
    // DRI2 认证失败（X 会话 3D 栈未接 nvidia），导致透明桌宠窗"存在但不可见"。
    // 因此默认一律启用软渲染兜底（桌宠/控制台为小窗口低负载 UI，软渲染无感知）；
    // 仅当显式 QIBAI_FORCE_HARDWARE=1 且 NVIDIA 驱动在位 + 本地图形会话（非 SSH）
    // 时才跳过兜底、走硬件 EGL。
    // 兜底变量：WEBKIT_DISABLE_DMABUF_RENDERER / WEBKIT_DISABLE_COMPOSITING_MODE /
    // LIBGL_ALWAYS_SOFTWARE / MESA_LOADER_DRIVER_OVERRIDE=llvmpipe。
    #[cfg(target_os = "linux")]
    {
        let force_hardware = std::env::var_os("QIBAI_FORCE_HARDWARE")
            .map(|v| v == "1")
            .unwrap_or(false);
        let nvidia_proprietary =
            std::path::Path::new("/proc/driver/nvidia/version").exists();
        let has_gui_session = std::env::var_os("DISPLAY").is_some()
            || std::env::var_os("WAYLAND_DISPLAY").is_some();
        let ssh_session = std::env::var_os("SSH_CONNECTION").is_some()
            || std::env::var_os("SSH_TTY").is_some();
        let use_software = !(force_hardware
            && nvidia_proprietary
            && has_gui_session
            && !ssh_session);
        let x11_software = use_software
            && std::env::var_os("DISPLAY").is_some()
            && std::env::var_os("WAYLAND_DISPLAY").is_none();
        // v43：透明恢复为默认能力。v42 在 X11 软渲染下一律强制不透明，
        // 导致「背景 canvas 不能透明」的回归；现仅当显式 QIBAI_X11_SAFE_OPAQUE=1
        // 时才启用防叠影兼容窗，其余情况保持原生透明。
        let safe_x11_opaque = std::env::var_os("QIBAI_X11_SAFE_OPAQUE")
            .map(|v| v == "1")
            .unwrap_or(false);
        // v45.4 自动兜底（startup survival mode）：
        // 用户杀了 compiz / WM 缺失 / 合成器 not found 时，WebKitGTK 建 ARGB 窗会自己
        // SIGTRAP + core dumped（Rust 侧无 panic 文本）。启动期内联探一次 _NET_SUPPORTING_WM_CHECK
        // 与 _NET_WM_CM_S0，任一缺失则本次强制 transparent(false)，保证能起来。
        // 用户若显式 QIBAI_X11_SAFE_OPAQUE=0（强制透明）就尊重用户选择。
        let user_force_transparent = std::env::var_os("QIBAI_X11_SAFE_OPAQUE")
            .map(|v| v == "0")
            .unwrap_or(false);
        let mut auto_opaque = false;
        if user_force_transparent {
            eprintln!("[qibai] user forced transparent (QIBAI_X11_SAFE_OPAQUE=0) — 后果自负，不降级");
        } else if safe_no_compositor_probe() {
            eprintln!("[qibai] compositor probe skipped (QIBAI_SAFE_NO_COMPOSITOR_PROBE=1) — 保持透明默认");
        } else {
            let wm_ok = std::process::Command::new("xprop")
                .arg("-root")
                .arg("_NET_SUPPORTING_WM_CHECK")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains("_NET_SUPPORTING_WM_CHECK"))
                .unwrap_or(false);
            let cm_ok = std::process::Command::new("xprop")
                .arg("-root")
                .arg("_NET_WM_CM_S0")
                .output()
                .map(|o| {
                    let s = String::from_utf8_lossy(&o.stdout);
                    s.contains("_NET_WM_CM_S0") && s.contains("window #")
                })
                .unwrap_or(false);
            if !wm_ok || !cm_ok {
                auto_opaque = true;
                AUTO_DEGRADED_OPAQUE.store(true, Ordering::Relaxed);
                eprintln!(
                    "[qibai] compositor 缺失 → 已自动降级为不透明窗口（透明/圆角本机不可用）(wm={}, cm={})",
                    wm_ok, cm_ok
                );
            }
        }
        let native_transparency = !(x11_software && safe_x11_opaque) && !auto_opaque;
        PET_NATIVE_TRANSPARENCY.store(native_transparency, Ordering::Relaxed);
        eprintln!(
            "[qibai] transparency: {} (x11_software={}, override={})",
            if native_transparency {
                "native"
            } else {
                "safe-opaque"
            },
            x11_software,
            safe_x11_opaque
        );
        eprintln!(
            "[qibai] render: {} (nvidia={}, session={}, ssh={})",
            if use_software { "software" } else { "hardware" },
            nvidia_proprietary,
            has_gui_session,
            ssh_session
        );
        // v15：本地发声引擎状态（捆绑 espeak / 系统 espeak）——启动诊断日志
        eprintln!(
            "[qibai] local-tts: {} (bundle={}, system={})",
            if tts::detect_local_tts_available() { "available" } else { "unavailable" },
            tts::bundled_espeak().is_some(),
            tts::system_espeak_available(),
        );
        if use_software {
            if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
                std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            }
            // 无合成层模式（WEBKIT_DISABLE_COMPOSITING_MODE=1）此前被验证为「重影」元凶：
            // v15 起移除，恢复 v13 组合（DMABUF 禁用 + llvmpipe 软渲染）。
            // 拖动变白如仍出现，用 QIBAI_FORCE_HARDWARE=1 硬渲染对照。
            if std::env::var_os("LIBGL_ALWAYS_SOFTWARE").is_none() {
                std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
            }
            if std::env::var_os("MESA_LOADER_DRIVER_OVERRIDE").is_none() {
                std::env::set_var("MESA_LOADER_DRIVER_OVERRIDE", "llvmpipe");
            }
        }
    }
    tauri::Builder::default()
        // 系统通知插件
        .plugin(tauri_plugin_notification::init())
        // 系统剪贴板（迭代③ vault_copy 写/清）
        .plugin(tauri_plugin_clipboard_manager::init())
        // 开机自启插件（macOS 用 LaunchAgent；Linux/Windows 插件内部自动处理）
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        // 迭代⑥：自动更新暂不启用——当前无真实发布源（endpoint/pubkey 均为占位），
        // updater 已整体停用：插件移除 + 调用点清理（app.updater() 未注册时
        // 会 panic，见迭代⑥说明）；check_update 命令返回 disabled 状态。
        .setup(|app| {
            let sink = Box::new(TauriSink {
                app: app.handle().clone(),
            });
            let engine = Arc::new(Mutex::new(Engine::new(sink)));
            // v45.8：plugins 目录统一——内置 src-tauri/plugins + 用户 <app_data>/plugins
            // core 线据此扫描 tool/doc/expert 三类，废弃 qibai-core/skills 与 ~/.qibai_skills。
            {
                let exe = std::env::current_exe().unwrap_or_default();
                let builtin = exe.parent().unwrap_or(std::path::Path::new(".")).join("plugins");
                let user = app.path().app_data_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
                    .join("plugins");
                eprintln!("[qibai] boot: plugin_roots builtin={builtin:?} user={user:?}");
                if let Ok(mut eng) = engine.try_lock() {
                    eng.set_plugin_roots(builtin, user);
                }
            }
            // v45.8：启动扬声器热插拔监听（5s 重探，设备消失自动回落 default）。
            // 顺序：先从设置文件灌全部偏好（audio_output/search_engine/timeout 等）
            // → 装 toast 钩子（设备丢失时前端弹"已切回系统默认"）→ 再起 watcher。
            if let Ok(mut eng) = engine.try_lock() {
                eng.apply_saved_preferences();
            }
            let app_handle = app.handle().clone();
            qibai_core::tts::set_toast_hook(move |msg| {
                let _ = app_handle.emit("toast", msg);
            });
            eprintln!("[qibai] audio: applied saved device={}", qibai_core::tts::current_audio_output());
            qibai_core::tts::start_hotplug_watcher();
            // v45.3：把 AppHandle 登记进 panic hook，让 panic 能 emit 前端事件。
            let _ = PANIC_APP.set(app.handle().clone());
            // 注册播放事件转发：core 产生 PlayEvent → 前端 qibai://tts-playback（开始/结束/失败）
            // 与 qibai://tts-interrupted（barge-in）。载荷只含引擎名/时长，不含文本。
            {
                let (tx, rx) = std::sync::mpsc::channel::<tts::PlayEvent>();
                tts::set_playback_sender(tx);
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    for ev in rx {
                        let (event, payload) = match ev {
                            tts::PlayEvent::Started { engine, duration_ms } => (
                                "tts-playback",
                                serde_json::json!({"state":"playing","engine":engine,"duration_ms":duration_ms}),
                            ),
                            tts::PlayEvent::Done { engine, duration_ms } => (
                                "tts-playback",
                                serde_json::json!({"state":"done","engine":engine,"duration_ms":duration_ms}),
                            ),
                            tts::PlayEvent::Failed { engine, error } => (
                                "tts-playback",
                                serde_json::json!({"state":"failed","engine":engine,"error":error}),
                            ),
                            tts::PlayEvent::Interrupted { engine } => (
                                "tts-interrupted",
                                serde_json::json!({"engine":engine}),
                            ),
                        };
                        let _ = handle.emit(event, payload);
                    }
                });
            }
            // v18.8-fix5：启动版本行——真机核对是否已覆盖最新包。
            eprintln!("[qibai] qibai-tauri v18.8-fix28 (LLM请求超时兜底：流式30s/非流式45s，不再卡死)");
            // v45.5 boot 04：音频初始化前打——崩溃时最后一行能定位到这里。
            if safe_no_audio() {
                eprintln!("[qibai] boot 04: audio-probe skipped (QIBAI_SAFE_NO_AUDIO=1)");
            } else {
                eprintln!("[qibai] boot 04: audio-probe starting");
            }

            // v18.7：启动音频自检（真机定位无声/听不到）。每项打印实际探测结果：
            //   1) TTS：捆绑引擎(exe上溯+cwd) / 系统 espeak / 播放器；
            //   2) STT：whisper-cli 与 ggml 模型实际路径；
            //   3) 合成器：X11 Composite 是否可用（无合成器 → 透明窗移动重影/橙色框）。
            {
                let bundle = tts::bundled_espeak();
                let sys = if cfg!(target_os = "linux") { tts::system_espeak_available() } else { false };
                eprintln!(
                    "[qibai] audio tts: bundled={} system_espeak={}",
                    bundle.as_ref().map(|b| b.bin.display().to_string()).unwrap_or_else(|| "none".into()),
                    if sys { "yes" } else { "no" }
                );
                // P0-S：启动自检对齐实际转写路径（whisper-server 优先），三项分开打。
                let stt = whisper_engine::stt_engine_status();
                eprintln!(
                    "[qibai] audio stt: server={} cli={} model={} probe={} ({})",
                    if stt.server_found { stt.server_path.clone() } else { "none".into() },
                    if stt.cli_found { stt.cli_path.clone() } else { "none".into() },
                    if stt.model_found { stt.model_path.clone() } else { "none".into() },
                    if stt.probe_ok { "OK" } else { "FAIL" },
                    stt.probe_reason
                );
                if !stt.probe_ok {
                    eprintln!("[qibai] {}", stt.placement_guide.replace('\n', " | "));
                }
                let players: [&str; 6] = ["paplay", "aplay", "ffplay", "mpv", "play", "cvlc"];
                let found: Vec<&str> = players.iter().copied().filter(|x| tts_which_binary(x)).collect();
                eprintln!("[qibai] audio players: {}", if found.is_empty() { "none!".to_string() } else { found.join(",") });
                // 合成器探测（诊断提示，不改变行为）：
                // _NET_SUPPORTING_WM_CHECK = 窗口管理器在；_NET_WM_CM_S0(root) = 合成器在。
                // v45.4：QIBAI_SAFE_NO_COMPOSITOR_PROBE=1 时整段跳过（刚杀 WM 环境下 xprop 可能阻塞/异常）。
                if safe_no_compositor_probe() {
                    eprintln!("[qibai] compositor: probe skipped (QIBAI_SAFE_NO_COMPOSITOR_PROBE=1)");
                } else if let Ok(out) = std::process::Command::new("xprop").arg("-root").arg("_NET_SUPPORTING_WM_CHECK").output() {
                    let s_ok = String::from_utf8_lossy(&out.stdout).contains("_NET_SUPPORTING_WM_CHECK");
                    if !s_ok { eprintln!("[qibai] display: WM 探测失败（xprop 不可用或无 X）"); }
                    match std::process::Command::new("xprop").arg("-root").arg("_NET_WM_CM_S0").output() {
                        Ok(out) => {
                            let cm = String::from_utf8_lossy(&out.stdout);
                            let present = cm.contains("_NET_WM_CM_S0") && cm.contains("window #");
                            if present {
                                eprintln!("[qibai] compositor: active（透明窗由合成器逐帧合成，拖动旧位置应被自动覆盖）");
                            } else {
                                eprintln!("[qibai] compositor: MISSING — 透明窗拖动残影为无合成器 X11 固有行为；根治建议：启动合成器（如 `picom -b`），否则残影无法从应用侧消除");
                            }
                        }
                        Err(e) => eprintln!("[qibai] compositor: 探测失败（xprop 不可用?）: {e}"),
                    }
                }
            }

            // v16：技能库 / 专家库随包目录自动装配（src-tauri/plugins/{skills,experts}，
            // 与用户数据目录 plugins/ 合并；幂等、跳过坏包）。
            {
                let mut g = engine.try_lock().ok();
                if let Some(g) = g.as_mut() {
                    if let Ok(exe) = std::env::current_exe() {
                        // exe = src-tauri/target/debug/qibai-tauri → 上三级 = src-tauri
                        let bundle = exe
                            .parent()
                            .and_then(|p| p.parent())
                            .and_then(|p| p.parent())
                            .map(|p| p.join("plugins"));
                        if let Some(d) = bundle {
                            // 技能库 / 专家库：各自为「库目录」（其下每个子目录即插件包）
                            for lib in ["skills", "experts"] {
                                let n = g.scan_plugin_dir(&d.join(lib));
                                eprintln!("[qibai] plugins: scanned {} from {}", n, d.join(lib).display());
                            }
                        }
                        let nskills = g.skill_list().len();
                        eprintln!("[qibai] skills: {} 个技能就绪", nskills);
                    }
                }
            }

            // 契约 §3：创建主角色桌宠窗 pet-<active>（不存在兜底 gudian）。
            // 容错：透明无边框窗在部分合成器/远程/软渲染环境创建失败，
            // 失败则回退普通窗口重试；仍失败仅记录日志，不阻断启动（避免"闪退"）。
            let active = {
                let a = engine.try_lock().ok().map(|g| g.personas.active_persona().to_string()).unwrap_or_default();
                if a.is_empty() {
                    "gudian".to_string()
                } else {
                    a
                }
            };
            // v45：启动建窗统一走 create_pet_window——label=pet-stage、几何（右缘定位）、
            // visible(false)+on_page_load show 与切角色路径完全一致，杜绝启动窗落屏中央、
            // 切换后新窗落右缘造成的「两个人并排」。
            let pet_ready = Arc::new(AtomicBool::new(false));
            eprintln!(
                "[qibai] boot 02: window-create label={} transparent={} compositor_degraded={}",
                STAGE_LABEL,
                pet_native_transparency(),
                AUTO_DEGRADED_OPAQUE.load(Ordering::Relaxed),
            );
            let _pet_result = create_pet_window(app.handle(), &active);

            // 免提聆听（v14）：构造共享状态；settings 开启唤醒时启动麦克风常开监听
            // （失败仅记录日志，不阻断启动；可稍后在设置/语音里开启）。
            let listen = Arc::new(listener::ListenerShared::new());
            // v45.3：只读快照缓存——先空快照（None），首次刷新由 spawn 立即补一次。
            let read_cache = Arc::new(RwLock::new(ReadCache::default()));
            // v45.21：用户叫停标志（engine 锁外，见 AppState 注释）。
            let cancel_flag = Arc::new(AtomicBool::new(false));
            // setup 是同步块不能 await；spawn 一个 task 立即刷一次（启动期 engine 锁空闲，try_lock 必成功）。
            {
                let e = engine.clone();
                let c = read_cache.clone();
                tauri::async_runtime::spawn(async move {
                    force_refresh(&e, &c).await;
                });
            }
            // 后台每 3s 非阻塞快照一次。
            spawn_refresher(engine.clone(), read_cache.clone());
            eprintln!("[qibai] boot 03: read_cache ready");
            app.manage(AppState {
                engine: engine.clone(),
                pet_ready: pet_ready.clone(),
                listen: listen.clone(),
                gui: Arc::new(std::sync::Mutex::new(
                    qibai_core::screen::GuiGuard::default(),
                )),
                read_cache: read_cache.clone(),
                cancel: cancel_flag.clone(),
            });

            // v18.8-fix17：启动时按设置自动启用免提聆听（根治"重启后听不到"——
            // 此前监听只在控制台保存设置时启动，重启应用后从未自动恢复）。
            {
                let wake_on = engine
                    .try_lock()
                    .ok()
                    .map(|g| g.settings.voice.wake_enabled)
                    .unwrap_or(true);
                eprintln!("[qibai] 免提聆听启动设置: wake_enabled={}（v8-PTT：默认按住说话，免提由前端按钮控制）", wake_on);
                // v8-PTT：不再启动自动开启免提——双工链路未验证稳定，默认按住说话（PTT），
                // 前端「免提」按钮（set_wake_enabled）显式开启，避免回声环路默认生效。
            }

            // 窗口控制事件（v14）：语音「藏起来/现身」→ 主进程执行窗口隐藏/显示
            {
                let h = app.handle().clone();
                let h2 = h.clone();
                h.listen("qibai://pet-hide", move |_| {
                    let _ = pets_hide(h2.clone());
                });
            }
            {
                let h = app.handle().clone();
                let h2 = h.clone();
                h.listen("qibai://pet-show", move |_| {
                    let _ = pets_show(h2.clone());
                });
            }
            // v18.3 屏幕「眼睛 + 手」：语音意图 → 主进程执行（截图/键鼠/窗口）
            {
                let h = app.handle().clone();
                let h2 = h.clone();
                h.listen("qibai://screen-cmd", move |event| {
                    let payload: serde_json::Value = serde_json::from_str(event.payload())
                        .unwrap_or(serde_json::Value::Null);
                    let cmd = payload["cmd"].as_str().unwrap_or("");
                        match cmd {
                            "capture" => {
                                match screen_gui::capture_png_base64() {
                                    Ok((b64, w, hh)) => {
                                        if let Ok(win) =
                                            ensure_panel_window(&h2, 920.0, 680.0)
                                        {
                                            let _ = win.set_title("元气小小 · 屏幕");
                                        }
                                        let _ = h2.emit(
                                            "qibai://screen-shot",
                                            serde_json::json!({ "image": b64, "w": w, "h": hh }),
                                        );
                                    }
                                    Err(e) => {
                                        let _ = h2.emit(
                                            "qibai://pet-say",
                                            serde_json::json!({ "persona_id": "default", "text": format!("截图失败：{e}"), "ms": 4000 }),
                                        );
                                    }
                                }
                            }
                            "gui_set" => {
                                let st = h2.state::<AppState>();
                                let mut g = st.gui.lock().unwrap_or_else(|e| e.into_inner());
                                if payload["enabled"].as_bool().unwrap_or(false) {
                                    g.grant();
                                } else {
                                    g.revoke();
                                }
                            }
                            "click" => {
                                let st = h2.state::<AppState>();
                                let mut g = st.gui.lock().unwrap_or_else(|e| e.into_inner());
                                if !g.consume() {
                                    let _ = h2.emit(
                                        "qibai://pet-say",
                                        serde_json::json!({ "persona_id": "default", "text": "操作权限未开启：请先语音说「打开操作权限」，或直接在截图面板上点击目标位置。", "ms": 4000 }),
                                    );
                                    return;
                                }
                                drop(g);
                                let x = payload["x"].as_u64().unwrap_or(0) as u32;
                                let y = payload["y"].as_u64().unwrap_or(0) as u32;
                                match screen_gui::click(x, y) {
                                    Ok(()) => {
                                        let _ = h2.emit(
                                            "qibai://pet-say",
                                            serde_json::json!({ "persona_id": "default", "text": format!("点了一下 ({x}, {y})。"), "ms": 2000 }),
                                        );
                                    }
                                    Err(e) => {
                                        let _ = h2.emit(
                                            "qibai://pet-say",
                                            serde_json::json!({ "persona_id": "default", "text": format!("点击失败：{e}"), "ms": 4000 }),
                                        );
                                    }
                                }
                            }
                            "type" => {
                                let st = h2.state::<AppState>();
                                let mut g = st.gui.lock().unwrap_or_else(|e| e.into_inner());
                                if !g.consume() {
                                    let _ = h2.emit(
                                        "qibai://pet-say",
                                        serde_json::json!({ "persona_id": "default", "text": "操作权限未开启：请先语音说「打开操作权限」。", "ms": 4000 }),
                                    );
                                    return;
                                }
                                drop(g);
                                let text = payload["text"].as_str().unwrap_or("").to_string();
                                match screen_gui::type_text(&text) {
                                    Ok(()) => {
                                        let _ = h2.emit(
                                            "qibai://pet-say",
                                            serde_json::json!({ "persona_id": "default", "text": "输入完成。", "ms": 2000 }),
                                        );
                                    }
                                    Err(e) => {
                                        let _ = h2.emit(
                                            "qibai://pet-say",
                                            serde_json::json!({ "persona_id": "default", "text": format!("输入失败：{e}"), "ms": 4000 }),
                                        );
                                    }
                                }
                            }
                            "key" => {
                                let st = h2.state::<AppState>();
                                let mut g = st.gui.lock().unwrap_or_else(|e| e.into_inner());
                                if !g.consume() {
                                    let _ = h2.emit(
                                        "qibai://pet-say",
                                        serde_json::json!({ "persona_id": "default", "text": "操作权限未开启：请先语音说「打开操作权限」。", "ms": 4000 }),
                                    );
                                    return;
                                }
                                drop(g);
                                let keys = payload["keys"].as_str().unwrap_or("").to_string();
                                match screen_gui::key(&keys) {
                                    Ok(()) => {
                                        let _ = h2.emit(
                                            "qibai://pet-say",
                                            serde_json::json!({ "persona_id": "default", "text": format!("按了一下「{keys}」。"), "ms": 2000 }),
                                        );
                                    }
                                    Err(e) => {
                                        let _ = h2.emit(
                                            "qibai://pet-say",
                                            serde_json::json!({ "persona_id": "default", "text": format!("按键失败：{e}"), "ms": 4000 }),
                                        );
                                    }
                                }
                            }
                            "scroll" => {
                                let st = h2.state::<AppState>();
                                let mut g = st.gui.lock().unwrap_or_else(|e| e.into_inner());
                                if !g.consume() {
                                    let _ = h2.emit(
                                        "qibai://pet-say",
                                        serde_json::json!({ "persona_id": "default", "text": "操作权限未开启：请先语音说「打开操作权限」。", "ms": 4000 }),
                                    );
                                    return;
                                }
                                drop(g);
                                let dy = payload["dy"].as_i64().unwrap_or(0) as i32;
                                match screen_gui::scroll(dy) {
                                    Ok(()) => {
                                        let _ = h2.emit(
                                            "qibai://pet-say",
                                            serde_json::json!({ "persona_id": "default", "text": if dy > 0 { "向下滚了滚。" } else { "向上滚了滚。" }, "ms": 2000 }),
                                        );
                                    }
                                    Err(e) => {
                                        let _ = h2.emit(
                                            "qibai://pet-say",
                                            serde_json::json!({ "persona_id": "default", "text": format!("滚动失败：{e}"), "ms": 4000 }),
                                        );
                                    }
                                }
                            }
                            "activate_window" => {
                                let st = h2.state::<AppState>();
                                let mut g = st.gui.lock().unwrap_or_else(|e| e.into_inner());
                                if !g.consume() {
                                    let _ = h2.emit(
                                        "qibai://pet-say",
                                        serde_json::json!({ "persona_id": "default", "text": "操作权限未开启：请先语音说「打开操作权限」。", "ms": 4000 }),
                                    );
                                    return;
                                }
                                drop(g);
                                let title = payload["title"].as_str().unwrap_or("").to_string();
                                match screen_gui::activate_window(&title) {
                                    Ok(()) => {
                                        let _ = h2.emit(
                                            "qibai://pet-say",
                                            serde_json::json!({ "persona_id": "default", "text": format!("已切到「{title}」窗口。"), "ms": 2000 }),
                                        );
                                    }
                                    Err(e) => {
                                        let _ = h2.emit(
                                            "qibai://pet-say",
                                            serde_json::json!({ "persona_id": "default", "text": format!("切换窗口失败：{e}"), "ms": 4000 }),
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                });
            }
            if safe_no_audio() {
                eprintln!("[qibai] listener: skipped (QIBAI_SAFE_NO_AUDIO=1)");
            } else if engine
                .try_lock()
                .ok()
                .map(|g| g.settings.voice.wake_enabled)
                .unwrap_or(false)
            {
                match listener::start_listener(app.handle().clone(), engine.clone(), listen) {
                    Ok(()) => {
                        let ww = engine
                            .try_lock()
                            .ok()
                            .map(|g| g.settings.voice.wake_word.clone())
                            .unwrap_or_else(|| "元气小小".into());
                        eprintln!("[qibai] 免提聆听已启动（唤醒词：{ww}）");
                    }
                    Err(e) => eprintln!("[qibai] 免提聆听未启动（可稍后在设置/语音里开启）: {e}"),
                }
            }

            // 内容就绪心跳（契约 §渲染兜底）：前端立绘加载完成后会调 pet_ready。
            // v20.4 根治「双立绘重叠」：旧逻辑 12s 未收到就 close+重建，而无合成器
            // X11 下旧窗 close 不彻底 → 旧窗+新窗并存 = 两个完整立绘重叠（且前端
            // 此前从不报 ready，等于每次启动必重建）。现在：
            //   - 12s 未 ready 且窗口仍存活 → 只是加载慢：不 close 不重建，仅日志；
            //   - 仅当窗口已消失（异常关闭）才重建普通窗兜底。
            {
                let ready = pet_ready.clone();
                let handle = app.handle().clone();
                let label = STAGE_LABEL.to_string();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(12)).await;
                    if ready.load(Ordering::SeqCst) {
                        return;
                    }
                    if handle.get_webview_window(&label).is_some() {
                        eprintln!(
                            "[qibai] pet frontend not ready in 12s but window alive: keep transparent window (NO rebuild, avoid double-character overlay)"
                        );
                        return;
                    }
                    // 窗口已消失（异常关闭）→ 重建普通窗兜底。
                    match WebviewWindowBuilder::new(
                        &handle,
                        label.clone(),
                        WebviewUrl::App("pet.html".into()),
                    )
                    .inner_size(360.0, 380.0)
                    .title("元气小小·桌宠")
                    .decorations(true)
                    .always_on_top(true)
                    .resizable(false)
                    .build()
                    {
                        Ok(w) => {
                            let _ = w.show();
                            let _ = w.set_focus();
                            eprintln!(
                                "[qibai] pet window was gone, rebuilt plain window ok ({label})"
                            );
                        }
                        Err(e) => {
                            eprintln!("[qibai] pet rebuild FAILED: {e}");
                        }
                    }
                });
            }


            // 托盘常驻：无系统托盘/StatusNotifier 的环境（无头/部分桌面）创建失败
            // 只记录不阻断——托盘缺失不影响主功能。
            if let Err(e) = build_tray(app.handle()) {
                eprintln!("[qibai] tray unavailable: {e}");
            }

            // 提醒后台 tick
            spawn_reminder_tick(app.handle().clone(), engine);

            // v45.5 boot 05：setup 全部走完——崩溃时若最后一行不是 05，卡点就在 04→05 之间。
            eprintln!("[qibai] boot 05: ready");

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // 契约 §2.1 既有
            get_settings,
            pet_render_info,
            save_settings,
            list_audio_devices,
            send_message,
            stop_message,
            stop_state,
            get_conversation,
            get_skills_status,
            set_skill_enabled,
            skill_install_stage,
            skill_install_commit,
            skill_install_cancel,
            skill_uninstall,
            skill_read_doc,
            skill_permissions,
            set_audio_output,
            current_audio_output,
            list_output_devices,
            set_search_engine,
            current_search_engine,
            save_settings_partial,
            task_create,
            task_list,
            task_get,
            task_plan,
            task_confirm_plan,
            task_start,
            task_pause,
            task_resume,
            task_cancel,
            task_confirm_action,
            task_rerun_failed,
            task_rerun,
            task_set_parallel_max,
            kb_add_folder,
            kb_remove_folder,
            kb_reindex,
            kb_status,
            kb_embed_status,
            kb_embed_fetch,
            kb_search,
            approve_skill,
            mcp_list,
            mcp_upsert,
            mcp_remove,
            mcp_set_enabled,
            mcp_restart,
            mcp_test_call,
            set_autostart,
            get_autostart,
            // 契约 §2.2 qibai-core 补接线
            get_personas,
            get_active_persona,
            set_active_persona,
            set_persona_voice, // 迭代⑤：每角色专属音色
            rename_persona,
            get_social_state,
            get_social_events,
            // 契约 §2.3 窗口管理
            window_identity,
            get_desk_state,
            pet_summon,
            pet_dismiss,
            pet_set_locked,
            console_open,
            console_repaint,
            console_close, // v21.11.8：控制台关闭命令（前端曾调 close_console/console_close 均未注册）
            // v21.11 独立菜单窗（架构根治：无合成器残留）
            ctx_menu_open,
            ctx_menu_pick,
            ctx_menu_close,
            pets_show,
            pets_hide,
            app_exit,
            pet_ready,
            // 契约 §2.3.1 UI 面板（v18：气泡卡片 + 独立面板窗）
            ui_panel_show,
            ui_panel_close,
            chat_log_panel,
            skills_panel,
            experts_panel,
            // v18.3 屏幕「眼睛 + 手」
            screen_capture,
            screen_gui_state,
            screen_gui_set,
            screen_gui_ack,
            screen_click,
            screen_type,
            screen_key,
            screen_scroll,
            screen_activate_window,
            // 契约 §2.4 社交包装
            social_greet,
            social_pk,
            // 契约 §2.5 亲密度（§6）
            get_intimacy_levels,
            get_intimacy_state,
            intimacy_event,
            set_anniversary,
            clear_anniversary,
            check_gate,
            // 迭代①：全屏守护演出 + 代码检查
            guard_trigger,
            guard_skip,
            guard_system_action,
            check_code,
            // 迭代②：TTS 发声陪伴
            tts_speak,
            tts_engine_status,
            tts_test_tone,
            stt_status,
            stt_transcribe,
            // v11 PTT：按住说话走 Rust 直采（绕开 WebView 麦克风权限）
            ptt_start,
            ptt_stop,
            // 迭代③：本地密码保险箱（契约 docs/VAULT_SPEC.md §4）
            vault_status,
            vault_setup,
            vault_setup_keyring,
            vault_unlock,
            vault_unlock_system,
            vault_lock,
            vault_list,
            vault_search,
            vault_add,
            vault_update,
            vault_delete,
            vault_copy,
            vault_export,
            vault_import_payload,
            // 迭代⑥：手动检查更新（控制台设置页）
            set_wake_enabled,
            get_listen_state,
            check_update,
            expert_list,
            debug_log,
            pet_move_by,
            pet_snap_to_edge,
            close_console,
            pet_start_drag,
            pet_keep_on_top,
            pet_repaint,
            pet_surface_rebuild,
            pet_invalidate_rect,
            skill_list,
            skill_invoke,
            scan_plugin_library,
            expert_set_active,
            plugin_grant_permission,
            plugin_install,
            plugin_list,
            plugin_set_enabled,
            plugin_uninstall,
            connector_add,
            connector_remove,
            connector_list,
            skill_pack_install,
            skill_pack_install_bytes,
            skill_pack_export_default,
            skill_pack_export,
            skill_pack_list,
            skill_install_stage_bytes,
            skill_open_dir,
            open_external,
            audio_test_output,
            audio_diagnose,
            netsocial_block,
            netsocial_cache_clear,
            netsocial_city_souls,
            netsocial_connect,
            netsocial_disconnect,
            netsocial_events,
            netsocial_history,
            netsocial_like,
            netsocial_match,
            netsocial_poll,
            netsocial_register,
            netsocial_report,
            netsocial_roam,
            netsocial_send,
            netsocial_set_profile,
            netsocial_status,
            netsocial_topics,
            // v18.8-fix8：语音诊断面板
            voice_diag,
            clear_chat_history,
            clear_conversation_scoped,
            voice_ui_dispatch,
            // v30：打开本地工作区目录（~/.qibai_workspace，QIBAI_WORKSPACE 可覆盖）
            code_open_workspace,
            // v31：打开本地知识库目录（~/.qibai_kb，QIBAI_KB 可覆盖）
            code_open_kb,
        ])
        .run(tauri::generate_context!())
        .expect("error while running qibai-tauri");
    // v18.8-fix27：应用退出时停掉常驻 whisper-server（避免孤儿进程）。
    whisper_engine::shutdown_server();
}
