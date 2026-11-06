# 全屏守护演出（Guard）— 迭代①交付说明

「全屏守护演出」模块：久坐到期 / 伴侣主动触发后，打开全屏守护窗口，播放
「截图底图 → 图标碎裂特效 → 伴侣现身 → 气泡提醒 → 收尾动作」的完整演出。

本迭代交付物（**全部为新增文件，未改动任何既有文件**）：

| 文件 | 职责 |
| --- | --- |
| `qibai-core/src/guard.rs` | 纯逻辑状态机：设置 / 调度 / 平台动作层，无 GUI 依赖 |
| `src/guard.html` / `src/guard.css` / `src/guard.js` | 全屏覆盖层演出页面（独立可加载，浏览器 `file://` 直开可待机/演示） |
| `docs/GUARD.md` | 本文档 |

接线（`lib.rs` 挂 `pub mod guard;`、Tauri 命令、窗口创建）由后续集成代理完成。

---

## 1. 平台命令

### 1.1 截图命令（底图来源，真机验证项）

演出底图优先使用当前桌面截图（data URL 传给前端 `payload.screenshot`）。本沙箱无桌面 GUI，
以下命令均**未在沙箱实测**，需真机验证：

- **Linux**
  - `import -window root /tmp/shot.png`（ImageMagick）
  - `gnome-screenshot -f /tmp/shot.png`（GNOME 桌面）
  - `scrot /tmp/shot.png`（轻量方案）
  - Wayland 下上述 X11 方案可能截不到屏，需另选 PipeWire 门户截图。
- **Windows**（PowerShell 片段）：
  ```powershell
  Add-Type -AssemblyName System.Windows.Forms,System.Drawing
  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
  $bmp.Save("$env:TEMP\qibai_shot.png")
  ```
- **macOS**：`screencapture -x /tmp/shot.png`（`-x` 静音）；首次使用需
  「系统设置 → 隐私与安全性 → 屏幕录制」授权。

隐私红线：截图仅在本地合成 data URL 交给前端铺底，**不上传任何远端**；前端在
`payload.screenshot` 缺失时用纯 CSS 模拟桌面（渐变 + 窗口矩形，已标注"视觉占位"），
绝不读取真实文件。

### 1.2 息屏 / 锁屏命令（`guard.rs` 平台动作层）

命令字符串由 `qibai-core/src/guard.rs` 的 `platform_sleep_cmd()` /
`platform_lock_cmd()` 按 `cfg(target_os)` 返回（**真机验证项**）：

| 平台 | sleep（息屏/休眠） | lock（锁屏） |
| --- | --- | --- |
| Linux | `xset dpms force off` | `loginctl lock-session` |
| Windows | `rundll32.exe powrprof.dll,SetSuspendState 0,1,0` | `rundll32.exe user32.dll,LockWorkStation` |
| macOS | `pmset displaysleepnow` | `osascript -e 'tell application "System Events" to keystroke "q" using {command down, control down}'`（候补方案，需「辅助功能」权限） |

---

## 2. 沙箱受限项（如实标注）

- 当前沙箱**无桌面 GUI**：截图命令、息屏/锁屏命令均无法执行与验证，全部标记
  「真机验证项」；集成前不要默认它们一定成功，前端按 `{simulated, detail}` 返回做兜底展示。
- `src-tauri` 因缺 webkit2gtk 在沙箱不可编译（既有环境限制）；`qibai-core` 纯库
  已在独立 scratch crate 跑通全部单测（见回交记录）。
- 前端 Canvas 碎裂特效为纯视觉动画，不接触真实屏幕内容。

## 3. 浏览器 mock 验证路径

不依赖后端的冒烟方式：

1. 浏览器直接 `file://` 打开 `src/guard.html`（需同目录 `tauri-bridge.js`）。
2. 待机态显示「等待守护信号」；点击「浏览器演示：播放完整演出」即本地播放完整演出
   （CSS 模拟桌面 → 碎裂特效 → 伴侣现身 → 气泡 → 倒计时遮罩），不依赖后端。
3. Tauri 环境下演示按钮自动隐藏，页面只监听 `qibai://guard-start`。
4. 全程「跳过」按钮 / ESC 可调；演出结束（自然或跳过）前端调 `guard_skip` 收尾。

---

## 4. 与集成代理的契约

### 4.1 Tauri 命令（invoke）

| 命令 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `guard_trigger` | `{}`（空） | `{started: bool, reason?: string}` 或 `null` | 手动触发入口：内部调 `GuardScheduler::manual_trigger(now)`；冷却中/未启用时返回 `null` |
| `guard_skip` | `{reason: 'user' \| 'auto'}` | `'ok'` | 演出收尾：内部调 `GuardScheduler::skip`/`finish`（按 reason 区分），并 emit `qibai://guard-end` |
| `guard_system_action` | `{action: 'sleep' \| 'lock'}` | `{simulated: bool, detail: string}` | 用 `platform_sleep_cmd()`/`platform_lock_cmd()` spawn 执行；沙箱/权限不足时 `simulated:true` 并说明 |

### 4.2 前端事件（emit / listen）

| 事件 | payload 形状 | 时机 |
| --- | --- | --- |
| `qibai://guard-start` | `{ mode: 'light'\|'strong'\|'lock', hours: u32, message: string, end_action: 'none'\|'sleep'\|'lock'\|'countdown', countdown_secs: u32(60..300), perform_fullscreen: bool, persona_id: string, screenshot?: string(data URL) }` | 触发成功后打开/显示守护窗口并演出 |
| `qibai://guard-end` | `{ reason: 'user'\|'auto', end_action: 'none'\|'sleep'\|'lock'\|'countdown' }` | 演出结束（前端已调 `guard_skip` 后由后端 emit） |
| `qibai://guard-action` | `{ action: 'sleep'\|'lock', ok: bool, detail: string }` | `guard_system_action` 执行结果（成功/失败/模拟） |

> 注：`GuardStart` 的 Rust 序列化字段与上表一一对应（serde snake_case）；
> `persona_id` 由后端按当前人设下发，前端拼 `assets/personas/<persona_id>/character.png`，
> 加载失败回退 CSS 占位脸。

### 4.3 路由规则速查（`guard.rs` 已实现）

- `Light` 模式：任何来源 `perform_fullscreen=false`（仅通知，不打开守护窗口）。
- `Strong` / `Lock`：全屏演出。
- `Lock` 模式：`end_action` 强制为 `lock`。
- 演出期间可 `skip`，跳过不执行收尾动作；结束后 30 分钟冷却，冷却期内任何触发返回 `None`。
