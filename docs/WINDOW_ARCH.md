# 元气小小 Qibai — 双窗口架构契约（WINDOW ARCHITECTURE CONTRACT）

> 本文档是本阶段重构的**冻结规范**：后端（Rust）、前端（pet/console）、桥接层（tauri-bridge.js）、
> 集成验证（冒烟/文档/打包）四路工作均以此为准，任何实现不得偏离本契约。
> 需求来源：用户原话「桌面宠物应该是独立于页面的一个存在」——桌宠窗是独立常驻存在，
> 功能页面迁移到按需呼出的控制台窗，后端（qibai-core）为唯一状态源。

---

## 0. 目标架构总览

```
┌─────────────────────────────────────────────────────────────┐
│                    qibai-core（唯一状态源）                    │
│  角色状态 / 亲密度 / 好感度 / 事件日志 / 提醒 / 技能 / desk 状态 │
└──────────────────────────┬──────────────────────────────────┘
                           │ IPC（invoke）+ 事件广播（emit）
        ┌──────────────────┴───────────────────┐
        ▼                                      ▼
┌───────────────────┐                 ┌────────────────────────┐
│  桌宠窗（每角色一窗） │                 │     控制台窗 console    │
│  pet-<persona_id>  │  经后端协调       │  （按需呼出，可关闭）     │
│  · 透明无边框置顶     │ ←──串门/PK────→ │  · 聊天 / 设置 / 形象   │
│  · 角色+气泡+拖动     │                 │  · 角色管理 / 事件日志   │
│  · 右键菜单           │                 │  · PK 入口 / 技能审批   │
│  · 无任何功能页面      │                 │  · 关闭不影响桌宠       │
└───────────────────┘                 └────────────────────────┘
```

核心原则：
1. **桌宠窗独立于控制台窗存在**：控制台开/关不影响任何桌宠窗的显示与状态。
2. **桌宠窗不承载功能页面 UI**：只渲染角色 + 气泡 + 拖动 + 右键菜单。
3. **后端统一协调**：召唤/送走/串门/PK 的窗口创建、事件广播、状态变更全部由 Rust 后端驱动；
   前端只响应事件、发 invoke，不做跨窗口直连。
4. **数据兼容幂等**：现有 social.json / personas.json / settings.json 无需迁移动作即可加载；
   新增字段带默认值，旧文件读入自动补齐。

---

## 1. 窗口清单

| 窗口 label | URL | 尺寸/配置 | 说明 |
|---|---|---|---|
| `pet-<persona_id>`（如 `pet-gudian`） | `pet.html` | 220×260；resizable=false；transparent=true；decorations=false；alwaysOnTop=true；shadow=false；**skipTaskbar=true** | 每在场角色一窗。运行时由后端创建（tauri.conf 不预声明） |
| `console` | `console.html` | 460×680；resizable=true；decorations=true；transparent=false；alwaysOnTop=false；skipTaskbar=false | 按需创建/关闭；关闭即销毁，重开由后端重新创建（状态全在 qibai-core） |

- `tauri.conf.json` `app.windows` 改为 **空数组**（所有窗口运行时创建）。
- 窗内身份：前端通过命令 `window_identity()` 拿 `{label}`，从 `pet-<id>` 解析 persona_id；
  **Tauri 下 URL 不带 query**。浏览器 mock 下用 `?persona=<id>` + `window.__qibaiWindowLabel`（见 §7）。

---

## 2. IPC 命令契约（invoke）

### 2.1 既有命令（保持不动，已在 main.rs 接线）
`get_settings` / `save_settings` / `send_message` / `get_conversation` / `get_skills_status` /
`set_skill_enabled` / `approve_skill` / `set_autostart` / `get_autostart`

### 2.2 从 qibai-core 补接线（存在核心 API 但 main.rs 未注册——必须补齐，否则 Tauri 真机社交功能缺失）
| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `get_personas` | — | `{active_persona, personas:[…]}` | 引擎 `get_personas()` |
| `set_active_persona` | `{persona_id}` | `"ok"` | 引擎 `set_active_persona`；**后端同步 desk**（见 §4），旧主角色桌宠窗若不再在场则关闭，新主角色窗口不存在则创建；广播 `qibai://desk-changed` |
| `rename_persona` | `{persona_id, nickname}` | `"ok"` | 引擎 `rename_persona` |
| `get_social_state` | — | `{relations:[…], levels:[…]}` | 引擎 `get_social_state` |
| `get_social_events` | `{limit?}` | `{events:[…]}` | 引擎 `get_social_events` |

### 2.3 新增：窗口管理命令
| 命令 | 参数 | 返回 | 行为 |
|---|---|---|---|
| `window_identity` | —（命令签名带 `window: tauri::WebviewWindow`） | `{label:"pet-gudian"}` | 返回调用方窗口 label |
| `get_desk_state` | — | `{present:[…], locked:{…}}` | 引擎 `desk_state()`，JSON 形状见 §4 |
| `pet_summon` | `{persona_id}` | `"ok"` | 引擎 `desk_summon`（幂等）；若窗口 `pet-<id>` 存在则 show+focus，否则创建（位置错开，见 §3）；广播 `qibai://desk-changed` |
| `pet_dismiss` | `{persona_id}` | `"ok"` | 引擎 `desk_dismiss`；关闭对应桌宠窗（若存在）；广播 `qibai://desk-changed`；若 dismiss 的是主角色 → 返回错误 |
| `pet_set_locked` | `{persona_id, locked}` | `"ok"` | 引擎 `desk_set_locked`；广播 `qibai://desk-changed` |
| `console_open` | — | `"ok"` | 控制台窗口存在则 show+focus，否则创建 |
| `pets_show` / `pets_hide` | — | `"ok"` | 遍历 label 前缀 `pet-` 的窗口 show/hide（托盘用） |
| `app_exit` | — | `"ok"` | `app.exit(0)`（桌宠右键菜单「退出」用） |

### 2.4 新增：社交命令包装（后端协调跨窗口演出）
| 命令 | 参数 | 返回 | 行为 |
|---|---|---|---|
| `social_greet` | `{from, to}` | 同引擎 `social_greet`（`{lines, affinity_delta, event}`） | 调引擎 `social_greet(from, to, false)`；**对 lines 逐条广播 `qibai://pet-say`**（speaker→对应桌宠窗气泡，见 §5）；返回结果给调用方 |
| `social_pk` | `{a, b, move_a, move_b}` | 同引擎 `social_pk`（`{blocked, winner, loser, result, affinity_delta, lines, remaining_today}`） | 调引擎 `social_pk`；**广播 `qibai://pk-round`** 给双方桌宠窗（含双方手势与 lines）；若 `result` 非 blocked 且 lines 存在，逐条广播 `qibai://pet-say`；返回结果给调用方 |

> mock 层（tauri-bridge.js）必须与上述后端行为一致：`social_greet`/`social_pk` 在返回结果的同时，
> 通过 mock 跨窗口总线广播同样的 `qibai://pet-say` / `qibai://pk-round`（见 §7），保证浏览器模式可全链路冒烟。

---

## 3. 窗口创建细则（Rust，A 实现）

- `setup()`：读 `engine.get_personas()` 拿 `active_persona`，创建主角色桌宠窗 `pet-<active>`（不存在的兜底 `gudian`）。
- `WebviewWindowBuilder`：label、`WebviewUrl::App("pet.html".into())`、宽 220 高 260、`transparent(true)`、
  `decorations(false)`、`always_on_top(true)`、`resizable(false)`、`shadow(false)`、`skip_taskbar(true)`。
- 新桌宠窗初始位置错开：主角色默认居中偏右上；第 n 个召唤窗口 `x += 48*n, y += 36*n`（自加偏移即可，不做碰撞检测）。
- `console` 窗口：宽 460 高 680、`decorations(true)`（正常标题栏）、可缩放、非置顶、显示于任务栏。
- 关闭语义：桌宠窗由用户关闭 = 该角色**仍在 desk 状态**（重开/托盘显示时恢复，见 §6 受限项备注）；
  控制台窗关闭 = 销毁 webview，重开由 `console_open` 重建，状态全在 qibai-core，无丢失。

---

## 4. desk 状态（qibai-core 新增，唯一事实源）

### 4.1 数据形状（持久化于 social.json 新增字段）
```jsonc
"desk": {
  "present": ["gudian", "huopo"],   // 在场桌宠 persona_id 列表（含主角色）
  "locked":  { "gudian": false, "huopo": true }   // 每角色位置锁定标记
}
```

### 4.2 规则
- `present` 恒含主角色（`set_active_persona` 时：`present = [new] ∪ (present \ {old})`）。
- `pet_dismiss(主角色)` → 拒绝（错误返回；用户通过切换主角色更换形象）。
- 同一 persona_id 不可同时为主角色与客人（召唤命令对主角色幂等 no-op）。
- 序列化：SocialStore 结构体新增 `desk` 字段，`#[serde(default)]`；旧 social.json 无此字段时加载为
  `{present: [], locked: {}}` —— **迁移幂等**（旧文件直接读入，不重写即无损）。
- 引擎新增方法：`desk_state() -> Value`、`desk_summon(id)`、`desk_dismiss(id)`、`desk_set_locked(id, locked)`，
  均 `Result` 化；写操作后调用 `SocialStore::persist`。

### 4.3 引擎行为变更点
- `Engine::set_active_persona`：追加 desk 簿记（替换 present 中的旧主角色），写入后 persist。
- `Engine::get_personas` / `get_social_state` 返回形状不变（desk 独立于两者，经 `get_desk_state` 获取）。

### 4.4 新增单测（A 实现，qibai-core 测试数从 53 增加）
1. `desk_migration_defaults_when_absent`：无 desk 字段的旧 social.json 读入后 desk 为默认值。
2. `desk_summon_dismiss_roundtrip`：召唤/送走/persist/重载一致。
3. `desk_lock_roundtrip`：锁定标记持久化。
4. `set_active_persona_replaces_active_on_desk`：切换主角色后 present 正确替换。
5. `dismiss_active_persona_rejected`：送走主角色返回错误。

---

## 5. 事件契约（后端 emit，全部广播；前端按 persona 过滤）

| 事件名 | payload | 消费方行为 |
|---|---|---|
| `qibai://reminder` | `{type, message, action}` | 全部桌宠窗：显示提醒 overlay + 播 action；控制台：toast |
| `qibai://llm-status` | `{state: thinking\|done\|error, message?}` | 控制台：聊天指示；桌宠窗：thinking 表情（focused） |
| `qibai://skill-approval` | `{request_id, skill, description}` | **控制台**：审批弹窗（桌宠窗不弹） |
| `qibai://desk-changed` | `{present:[…], locked:{…}}` | 控制台：刷新角色管理/关系列表；桌宠窗：自己不在 present 时（仅 mock 场景）显示道别气泡 |
| `qibai://pet-say` | `{persona_id, text, emotion?, action?, ms?}` | **仅 persona_id 匹配本窗**的桌宠窗：显示气泡 + 表情/动作 |
| `qibai://pk-round` | `{a, b, move_a, move_b, result, winner, lines:[{speaker,text}]}` | 双方桌宠窗：手势动画（本窗=自己的 move）+ 结果情绪；控制台：显示结果文本 |
| `qibai://pk-end` | `{a, b}` | 双方桌宠窗：退出 PK 演出态 |
| `qibai://toast` | `{text, persona_id?}` | 控制台：toast；带 persona_id 时对应桌宠窗显示气泡 |

- 路由策略：**后端一律 `app.emit` 全局广播，前端按 persona_id 过滤**（简单、无路由 bug；TauriSink 接口不变）。
- 桌宠窗对 `qibai://pet-say` 必须校验 `payload.persona_id === 本窗 persona` 才演出，否则忽略。

---

## 6. 前端文件布局

```
src/
  pet.html / pet.js / pet.css        ← 桌宠窗（B 新建；桌宠逻辑自 app.js 抽取，去功能 UI）
  console.html / console.js / console.css  ← 控制台窗（C 新建；合并 settings.js + app.js 控制台侧逻辑）
  tauri-bridge.js                    ← 桥接层（Organizer 扩展，B/C 只读不改！）
  styles.css                         ← 共享基础样式（既有，B/C 只新增 pet.css / console.css，不动 styles.css）
  assets/                            ← 5 形象资产（不改）
  删除（由集成 Agent D 统一执行）：index.html / settings.html / settings.js / app.js
```

- **文件所有权**：B 只写 `pet.html / pet.js / pet.css`；C 只写 `console.html / console.js / console.css`；
  两人都不得编辑 `tauri-bridge.js` / `styles.css` / 彼此文件；删除与全局查引用归 D。
- 桥接层对外 API（B/C 消费）：`QibaiBridge.{invoke, listen, isTauri, windowLabel}`；
  `QibaiBridge.windowLabel`：Tauri 下由 `window_identity()` 异步获取并缓存；mock 下为
  `window.__qibaiWindowLabel`（pet.html 在浏览器模式内联设置，见 §7）或 `'browser'`（控制台/harness）。
- 视觉：沿用现有 styles.css 的色板与组件语言（粉色系 #FBD3E4/#D17397、卡片、开关、toast 等），
  不引入新配色体系；图标用内联 SVG；不用 emoji；控件热区 ≥44px；输入框 ≥16px。
- 冒烟 hash 钩子（浏览器模式专用，Tauri 无副作用）：
  - `pet.html?persona=gudian#fire=wave|happy|reminder-sit|menu`（menu=打开右键菜单）
  - `console.html#tab=chat|roles|events|settings|pk&fire=...`（各 tab 的确定性演示）

### 6.1 桌宠窗页面结构（B，pet.html）
- 顶栏拖拽条（`data-tauri-drag-region`）+ 整窗可拖区域（锁定时禁用拖拽响应）。
- 主气泡（speech bubble）+ 表情系统 + 动作系统 + 眨眼（复用 app.js 既有实现，适配单角色）。
- 右键菜单（自定义 HTML 菜单）：**打开控制台** / **锁定位置（切换）** / **退出元气小小**。
- 提醒 overlay（复用 styles.css 既有类）。
- 浏览器 mock 调试面板（仅浏览器模式，收窄为：表情/动作/提醒 触发 + 打开控制台按钮）。
- **严禁出现**：聊天输入框、工具栏、设置链接、PK overlay、事件面板、技能审批弹窗。

### 6.2 控制台窗页面结构（C，console.html）
- 顶部：标题 + 关闭按钮（Tauri `window.close()`；mock 下 toast「控制台已关闭（浏览器演示）」，由 harness 控制显示）。
- Tab 栏：**聊天 / 角色 / 事件 / 设置 / 猜拳**。
  - 聊天：会话列表（`get_conversation`）+ 输入发送（`send_message`）+ thinking 指示 + 技能审批弹窗（全局 overlay）。
  - 角色：形象网格（切换=set_active_persona）+ 角色管理列表（设为主角色/召唤/送走/重命名）+
    角色关系（好感度/等级/今日 PK 次数，`get_social_state`）。
  - 事件：事件日志（`get_social_events`，时间倒序）。
  - 设置：LLM 配置 + 提醒 + 隐私模式 + 技能授权（settings.html 各 section 原样迁移，`save_settings`）。
  - 猜拳：互动区（在场客人选择器 + 「串门寒暄」按钮 + 「猜拳」按钮 + 手势按钮 + 结果区），
    串门走 `social_greet`、猜拳走 `social_pk`（结果与台词展示在本区，动画在桌宠窗）。
- 监听事件：`qibai://desk-changed`（刷新角色/关系）、`qibai://skill-approval`（弹窗）、
  `qibai://reminder`/`qibai://toast`（toast）、`qibai://llm-status`（聊天指示）、`qibai://pk-round`（结果区）。

---

## 7. mock 跨窗口总线协议（tauri-bridge.js，Organizer 扩展）

浏览器多窗口模拟 = **共享 localStorage + `storage` 事件**（Chrome file:// 同源页面共享 localStorage）。

### 7.1 身份
- pet.html 在 `<script>`（bridge 加载前）内联：
  ```html
  <script>
    if (!window.__TAURI_INTERNALS__) {
      var m = /persona=([a-z]+)/.exec(location.search);
      window.__qibaiWindowLabel = 'pet-' + (m ? m[1] : 'gudian');
    }
  </script>
  ```
- 控制台/harness：`QibaiBridge.windowLabel = 'console' | 'harness'`（默认 `'browser'`）。

### 7.2 跨窗口事件（cross-bus）
- 写入：`localStorage.setItem('qibai.mock.bus', JSON.stringify({event, payload, ts, from: windowLabel}))`。
- 读取：各页 `window.addEventListener('storage', e => { if (e.key==='qibai.mock.bus') … })`，
  解析后分发给本页 `listen(event, handler)` 注册的监听器（与同页 `mockEmit` 合并，统一走 `listen()` 出口）。
- **同页监听器在 storage 事件里不触发**（浏览器语义），故同页直接 `mockEmit`、跨页走总线，两路都进监听器。

### 7.3 mock 状态与命令语义（与后端对齐）
- `qibai.mock.desk`：`{present, locked}`；默认 `present:[active_persona]`、`locked:{}`（首次由读取方初始化）。
- `pet_summon`：写 desk + 经总线广播 `qibai://desk-changed`（不创建窗口——窗口由 smoke harness 监听
  desk-changed 挂载/卸载 iframe）。
- `pet_dismiss` / `pet_set_locked` / `set_active_persona`：同步 desk + 广播 `qibai://desk-changed`。
- `console_open`：经总线广播 `qibai://console-open`（harness 监听后显示 console iframe）；
  mock 内同页直接调用时同样走总线（保证 harness 收到）。
- `social_greet`：复用现有 mock 计算逻辑；返回 `{lines, affinity_delta, event}` 的同时，
  逐条经总线广播 `qibai://pet-say`（speaker→对应桌宠窗）。
- `social_pk`：复用现有 mock 计算逻辑；返回结果的同时经总线广播 `qibai://pk-round` +
  逐条 `qibai://pet-say`。
- `window_identity` mock：返回 `{label: QibaiBridge.windowLabel}`。
- 事件去重：总线每条带 `ts`，各页记录已处理 `ts` 集合，重复忽略（storage 事件有跨 frame 复制风险）。

---

## 8. 冒烟计划（D 执行）

### 8.1 浏览器 mock 冒烟 harness（src/smoke.html）
- 三 iframe 槽：`pet-gudian`（常挂）、`pet-huopo`（随 desk-changed 挂载/卸载）、`console`（随 console-open 挂载/卸载）。
- 顶部控制条按钮（全部经 `QibaiBridge.invoke` 驱动，不直接操纵 iframe——file:// 跨文档不可脚本化）：
  打开/关闭控制台、召唤火火、送走火火、串门寒暄、猜拳一局、切换主角色。
- 确定性模式（hash）：`smoke.html#mode=dual`（gudian+huopo 双桌宠 + 控制台开）、
  `#mode=pk`（执行猜拳后定格结果）、`#mode=greet`（串门台词演出后定格）。

### 8.2 截图清单（存 qibai/artifacts/，命名前缀 `dualwin_`）
1. `dualwin_pet_standalone.png`：`pet.html?persona=gudian#fire=wave` —— 桌宠窗独立存在、无功能 UI。
2. `dualwin_console_closed.png`：smoke.html 初始（仅 pet-gudian iframe，控制台槽隐藏）—— 桌宠独立于控制台。
3. `dualwin_console_open.png`：打开控制台后 —— 控制台与桌宠并存，桌宠不受影响。
4. `dualwin_two_pets.png`：召唤火火后 —— 双桌宠窗并存。
5. `dualwin_pk.png`：`#mode=pk` —— 双桌宠手势动画 + 结果（跨窗口互动）。
6. `dualwin_greet.png`：`#mode=greet` —— 双桌宠跨窗口台词气泡。
7. `dualwin_menu.png`：`pet.html#fire=menu` —— 桌宠右键菜单（打开控制台/锁定/退出）。

### 8.3 自检项
- qibai-core `cargo test -p qibai-core` 全过（53 + 新增）。
- src-tauri 沙箱无法编译（缺 gdk-3.0/webkit2gtk-4.1）→ `rustfmt --check` 语法自检 +
  逐条对照本契约人工复核；受限项如实标注、给真机验证路径（docs/BUILD_NOTE.md）。
- 浏览器模式：上述 7 张截图 + 页面无 JS 崩溃（GUI 浏览器打开各页确认渲染正常）。
- 悬空引用：grep `index.html|settings.html|settings.js|app.js` 于 README/CHECKLIST/SMOKE_TEST/docs 与 src/，全部更新/清除。

---

## 9. 验收对照（用户/主代理验收标准 → 本契约落点）

| 验收标准 | 落点 |
|---|---|
| 桌宠窗独立于控制台窗存在；控制台开/关不影响桌宠显示与状态 | §1 双窗口；§3 关闭语义；§8.2 截图 2/3；状态源在 qibai-core（§4） |
| 桌宠窗无功能页面 UI | §6.1 严禁清单；截图 1 |
| 控制台经托盘/桌宠右键呼出、可关闭；关闭后桌宠仍在 | §2.3 `console_open`；托盘菜单重做；桌宠右键「打开控制台」；截图 3 |
| 召唤第二角色产生独立桌宠窗；串门/PK 跨窗口可用且有冒烟截图 | §2.3/2.4；§5 pk-round/pet-say；截图 4/5/6 |
| qibai-core 测试全过；文档更新；新 zip（排除 target） | §8.3；D 更新 README/CHECKLIST/IPC_CONTRACT/SMOKE_TEST + 本契约；zip 规则见 §10 |
| 受限项如实标注并给替代验证路径 | §8.3 src-tauri 编译限制；docs/BUILD_NOTE.md |

## 10. 打包规则（D）

- `zip -r 元气小小_MVP_源码包.zip <qibai 全部> --exclude '*/target/*' '*/target' 'qibai/target/*'`，
  确保不含 `qibai/target` 与 `src-tauri/target`；含全部变更 + 本契约文档。
- 复制到 `/home/user/Doubao/chats/38448289926008322/元气小小_MVP_源码包.zip`（覆盖旧包）。
- 保留旧 zip 不做迁移（覆盖即交付新版本）。
