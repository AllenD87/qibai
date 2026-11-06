# 元气小小 Qibai — 前后端 IPC 契约

前端通过 Tauri `invoke` 调用后端命令，后端通过 `event.emit` 向前端推送事件。
本文件完整列出契约，供前端对照实现。所有 JSON 字段均已与 `qibai-core` 源码对齐。

> **v20 架构审查修正注记（2026-10-26）**：经 Rust↔前端↔文档三方对账，发现以下漂移，本注记为权威补充——
>
> **A. 文档列了但 Rust 当前从未 emit 的事件（预留未实现，前端死订阅应清理）**：
> - `qibai://pk-end`、`qibai://toast`、`qibai://console-open`：下文 §二.7/8/9 所列为规划项，当前 Rust 侧无 emit。前端 console.js 对前两者的 listen 永远等不到事件，前端官清理。
>
> **B. Rust 已 emit 但本文未记录的事件（补充）**：
> - `qibai://conversation-updated`（聊天记录变更后广播，console/pet 刷新用）
> - `qibai://tts-chunk`（流式逐段朗读文本段）
> - `qibai://bubble-dismiss`、`qibai://pet-hide`、`qibai://pet-show`
> - `qibai://panel-content`、`qibai://screen-cmd`、`qibai://screen-shot`、`qibai://ui-panel`
>
> **C. v20 新增命令**：
> - `pet_snap_to_edge()` → `{snapped:bool, edge:'left'|'right'|'top'|'bottom'|'none', x:number, y:number}`（桌宠边缘吸附，threshold=14px 在 Rust 侧，按 monitor work_area）
> - `netsocial_cache_clear()` → 删除联网社交加密缓存并重置身份

---

## 一、invoke 命令

### 1. `get_settings()` → `SettingsJson`
读取当前设置。

```jsonc
{
  "llm": {
    "provider": "doubao",
    "base_url": "https://ark.cn-beijing.volces.com/api/v3/chat/completions",
    "model": "doubao-pro-32k",
    "api_key": ""            // 可为空字符串；空时后端读环境变量 DOUBAO_API_KEY
  },
  "reminders": {
    "sit_min": 45,           // 久坐间隔（分钟）
    "water_min": 30,         // 喝水间隔
    "eye_min": 20,           // 护眼间隔
    "enabled": { "sit": true, "water": true, "eye": true }
  },
  "ui": {
    "age_confirmed": false,  // 用户是否已确认成年
    "pure_mode": false       // 未成年人纯净模式
  },
  "skills": {
    "total_enabled": true,   // 技能总开关（一键停用全部授权）
    "launch_app": true,
    "open_folder": true,
    "set_reminder": true
  },
  "intimacy_level": 1        // 亲密度 L1-L10，本期仅预留字段
}
```

### 2. `save_settings(settings: SettingsJson)` → `"ok"`
整表覆盖写入设置（JSON 落到 `<data_dir>/settings.json`）。

### 3. `send_message({ text: string })` → `SendMessageResult`
```jsonc
{
  "reply": "已经帮你打开浏览器啦",   // 给用户看的话
  "emotion": "happy",              // 见 emotion 枚举
  "action": "wave",                // 可选；见 action 枚举
  "skill": "launch_app"            // 可选；本轮对话实际执行的技能名
}
```
错误情形：未配置 API key 时返回结构化错误 `MissingApiKey`
（message 形如 `missing API key: set env DOUBAO_API_KEY or settings.llm.api_key`）。

### 4. `get_conversation()` → `Array<{ role, content, ts }>`
```jsonc
[
  { "role": "user",      "content": "打开浏览器", "ts": 1730000000 },
  { "role": "assistant", "content": "好的",       "ts": 1730000001 }
]
```

### 5. `get_skills_status()` → `SkillsStatus`
```jsonc
{
  "total_enabled": true,
  "skills": {
    "launch_app": true,
    "open_folder": true,
    "set_reminder": true
  }
}
```

### 6. `set_skill_enabled({ skill: string, enabled: boolean })` → `"ok"`
`skill` ∈ `["launch_app", "open_folder", "set_reminder"]`。
未在白名单内的技能名会返回错误。

### 7. `approve_skill({ request_id: string, approve: boolean })` → `string`
前端用户对危险操作（如 `execute_command`）做确认后回调。
`approve=true` 执行；`false` 取消。返回一段文本说明结果。

### 8. `set_autostart({ enabled: boolean })` → `boolean`
开启/关闭开机自启。返回当前是否已启用。
底层由 `tauri-plugin-autostart` 实现：Windows 注册表 / macOS LaunchAgent / Linux `~/.config/autostart/*.desktop`。

### 9. `get_autostart()` → `boolean`
查询当前开机自启是否已启用。

---

## 一·补、P0+P1 多角色社交命令（本期新增）

> 完整设计与冻结契约见 [`P0P1_SOCIAL_PLAN.md`](P0P1_SOCIAL_PLAN.md)。
> 以下 7 个命令在浏览器 mock 模式（`tauri-bridge.js` 的 localStorage 假实现）与 Tauri 真机环境中**同名同形**，
> 出拳在前端、判定/计分/防刷在 core；所有失败路径返回结构化错误，不 panic。

### 10. `get_personas()` → `PersonasJson`

```jsonc
{
  "active_persona": "gudian",
  "personas": [
    {
      "persona_id": "gudian",            // 枚举：gudian / dushi / wenrou / huopo / reqing
      "display_name": "古典佳人",         // 注册表显示名
      "nickname": "元气小小",                // 用户可重命名
      "traits": { "tone": "温婉", "catchphrase": "……", "interaction": "……" },
      "intimacy": 1,                    // 与用户的亲密度 1–10（迁移自旧 settings.intimacy_level）
      "memory": [ { "role": "user", "content": "…", "ts": 0 } ],  // 独立记忆，上限 50 条环形裁剪
      "sync": { "server_id": null, "dirty": false, "version": 0 } // 远期联网预留，本期只写不读
    }
  ]
}
```

### 11. `set_active_persona({ persona_id: string })` → `"ok"`
切换当前激活角色。`persona_id` 须为上述 5 个枚举之一，未知值返回结构化错误 `UnknownPersona`。

### 12. `rename_persona({ persona_id: string, nickname: string })` → `"ok"`
重命名指定角色的昵称，写入 `personas.json`；未知 `persona_id` 返回结构化错误。

### 13. `get_social_state()` → `SocialStateJson`

```jsonc
{
  "relations": [
    {
      "a": "gudian", "b": "huopo",        // 无序对，a<b 字典序归一
      "affinity": 0,                      // 角色间好感度
      "level": 0,                          // 0–3，见等级表
      "title": "初识",
      "pk_count": { "date": "2026-10-21", "count": 0 },  // 每日每对 PK 次数（防刷，上限 3）
      "greet_today": 0, "greet_date": "2026-10-21",    // 每日串门计分次数（上限 5）
      "summon_count": 0, "greet_count": 0, "pk_total": 0,
      "last_interaction_ts": 0
    }
  ],
  "levels": [                             // 交往 4 级表（固定 4 档）
    { "level": 0, "name": "初识",        "title": "互称全名", "min": 0,   "max": 99 },
    { "level": 1, "name": "朋友",         "title": "昵称",     "min": 100, "max": 299 },
    { "level": 2, "name": "闺蜜/搭档",   "title": "专属昵称", "min": 300, "max": 599 },
    { "level": 3, "name": "羁绊",        "title": "挚友宣言", "min": 600, "max": null }
  ]
}
```

### 14. `social_greet({ from: string, to: string, use_llm?: boolean })` → `GreetResult`

```jsonc
{
  "lines": [ { "speaker": "gudian", "text": "……" } ],   // speaker 为该轮台词所属 persona_id，前端据此决定哪个角色气泡发声
  "affinity_delta": 1,                                 // 每日每对前 5 次 +1，之后计 0（防刷封顶）
  "event": {                                            // 同时写入事件日志
    "ts": 1730000000, "type": "greet", "a": "gudian", "b": "huopo",
    "detail": { "affinity_delta": 1, "capped": false, "level": 0 }
  }
}
```

- 寒暄台词模板优先（按 persona 对 × 关系等级分档的内置剧本）；
- 传 `use_llm=true` 且配置了 API Key 时走单轮 LLM 轻量生成；**无 key 自动忽略该参数降级模板**。

### 15. `social_pk({ a: string, b: string, move_a: string, move_b: string })` → `PkResult`

```jsonc
{
  "winner": "huopo",        // 胜方 persona_id；平局为 null
  "loser": "gudian",        // 败方 persona_id；平局为 null
  "result": "win",          // win | draw
  "affinity_delta": 10,     // 胜局角色对 +10、平局双方 +4
  "lines": [ { "speaker": "huopo", "text": "我出布，哈哈，赢啦！" } ],
  "remaining_today": 2,     // 今日剩余 PK 次数（每日每对 3 次）
  "blocked": false          // true = 今日次数用尽
}
```

- `move` 枚举（前端中文显示"石头 / 剪刀 / 布"，传输用英文 key）：`rock | paper | scissors`；
- `blocked=true` 时仅返回 `{ "blocked": true, "remaining_today": 0, "lines": [提示文案] }`，其余字段省略；
- 跨档升级时 `lines` 末尾追加解锁彩蛋台词，并写入一条 `level_up` 事件。

### 16. `get_social_events({ limit?: number })` → `EventsResult`

```jsonc
{ "events": [ { "ts": 1730000000, "type": "greet|pk|level_up|summon|dismiss", "a": "gudian", "b": "huopo", "detail": {} } ] }
```

时间倒序返回；`limit` 缺省 100，上限 200 条环形裁剪。
事件 `type ∈ { greet, pk, level_up, summon, dismiss }`（`summon` / `dismiss` 为前端召唤状态变化的记录入口）。

---

## 一·双补、双窗口架构命令（WINDOW_ARCH.md §2）

> 冻结契约：[`WINDOW_ARCH.md`](WINDOW_ARCH.md)。
> 桌宠窗（`pet-<persona_id>`，每在场角色一窗）与控制台窗（`console`，按需呼出）相互独立；
> 召唤/送走/串门/PK 的窗口创建与事件广播全部由后端协调，前端只响应事件、发 invoke。

### 17. 补接线命令（qibai-core 已存在 API，main.rs 注册）

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `get_personas` | — | `{active_persona, personas:[…]}` | 同上文 §10（重复列出便于双窗口章节对照） |
| `set_active_persona` | `{persona_id}` | `"ok"` | 切换激活角色；**后端同步 desk 在场名单**：`present = [new] ∪ (present \ {old})`，广播 `qibai://desk-changed` |
| `rename_persona` | `{persona_id, nickname}` | `"ok"` | 重命名昵称 |
| `get_social_state` | — | `{relations:[…], levels:[…]}` | 同上文 §13 |
| `get_social_events` | `{limit?}` | `{events:[…]}` | 同上文 §16 |

### 18. 窗口管理命令

| 命令 | 参数 | 返回 | 行为 |
|---|---|---|---|
| `window_identity` | — | `{label:"pet-gudian"}` | 返回调用方窗口 label（Tauri 下由 `WebviewWindow` 注入；mock 下读 `window.__qibaiWindowLabel`） |
| `get_desk_state` | — | `{present:[…], locked:{…}}` | desk 在场名单（持久于 `social.json` 的 `desk` 字段） |
| `pet_summon` | `{persona_id}` | `"ok"` | 引擎 `desk_summon`（幂等）；窗口已存在则 show+focus，否则创建（位置错开）；广播 `qibai://desk-changed` |
| `pet_dismiss` | `{persona_id}` | `"ok"` | 引擎 `desk_dismiss`；关闭对应桌宠窗；广播 `qibai://desk-changed`；**送走主角色返回错误** |
| `pet_set_locked` | `{persona_id, locked}` | `"ok"` | 位置锁定标记；广播 `qibai://desk-changed` |
| `console_open` | — | `"ok"` | 控制台窗口存在则 show+focus，否则创建；广播 `qibai://console-open` |
| `pets_show` / `pets_hide` | — | `"ok"` | 遍历 label 前缀 `pet-` 的窗口 show/hide（托盘用） |
| `app_exit` | — | `"ok"` | `app.exit(0)`（桌宠右键菜单「退出元气小小」用） |

### 19. 社交命令包装（后端协调跨窗口演出）

| 命令 | 参数 | 行为 |
|---|---|---|
| `social_greet` | `{from, to}` | 调引擎 `social_greet`；**对返回 lines 逐条广播 `qibai://pet-say`**（`{persona_id: speaker, text, ms}`，对应桌宠窗气泡演出）；返回结果给调用方 |
| `social_pk` | `{a, b, move_a, move_b}` | 调引擎 `social_pk`；**广播 `qibai://pk-round`**（`{a,b,move_a,move_b,result,winner,lines}`）给双方桌宠窗；非 blocked 时对 lines 逐条广播 `qibai://pet-say`；返回结果给调用方 |

> mock 层（`tauri-bridge.js`）与上述后端行为一致：同页 `mockEmit` + localStorage 总线广播 +
> 400ms 轮询兜底，跨 iframe/跨页按 `persona_id` 过滤演出。

---

## 一·叁补、亲密度 L1–L10 命令（INTIMACY_SYSTEM.md §6）

> 冻结契约：[`INTIMACY_SYSTEM.md`](INTIMACY_SYSTEM.md)。
> 每角色独立亲密度；门控判定统一走 qibai-core（`check_gate` / `approve_skill`），前端只读状态。

### 20. 亲密度命令（qibai-core `engine.rs` + `src-tauri/main.rs` 注册）

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `get_intimacy_levels` | — | `{levels:[{level,name,desc,min,max}×10]}` | L1–L10 等级表（阈值 `[0,120,280,480,720,1000,1330,1720,2180,2720]`，L10 max=null） |
| `get_intimacy_state` | `{persona_id}` | `{persona_id, points, level, level_name, progress:{min,max,ratio}, unlocked:[{gate,label,required_level,kind}], locked:[…], daily:{date,gained,cap:60}, cumulative:{chats,clicks,pk_wins,companion_days,greetings,gifts}, tasks:[{id,name,done}×3], achievements:[{id,name,ts}], anniversary:{month,day}|null, sync:{server_id:null,dirty:false,version:0}}` | 单角色完整状态；未知角色 → `UnknownPersona` 错误 |
| `intimacy_event` | `{persona_id, kind, detail?}` | `{delta, points, level, leveled_up, unlocked:[…], achievements_granted:[…], tasks_updated:[…], capped}` | kind ∈ `chat`(+2) / `click`(+1, 日限10) / `task`(+5, detail.task_id ∈ greeting\|click\|pk) / `companion`(9 档阈值递减) / `gift`(+10, 日限1) / `pk_win`(+0, 只记累计)；**每日总上限 60**，达到后 `capped=true`；升级写社会事件 `intimacy_level_up` 并广播 |
| `set_anniversary` | `{persona_id, month, day}` | `"ok"` | 纪念日设置；month 1–12 / day 1–31 非法 → 错误 |
| `clear_anniversary` | `{persona_id}` | `"ok"` | 清除纪念日 |
| `check_gate` | `{persona_id, gate}` | `{open, required_level, current_level}` | 门控判定（21 键）；未知 persona 宽容返回 `current_level=1` |

### 21. 门控键表（21 键，kind: active | placeholder）

| gate | 所需等级 | 解锁内容 | kind |
|---|---|---|---|
| basic_chat / idle_action / sit_reminder | L1 | 基础对话 / 待机动作 / 久坐提醒 | active |
| nickname / daily_greeting / water_reminder | L2 | 自定义称呼 / 每日问候 / 喝水提醒 | active |
| remember_prefs / boot_greeting | L3 | 记住偏好（作息/常用软件）/ 开机问候 | active |
| assistant_skills | L4 | 助理技能（打开应用/文件夹、设提醒） | active |
| vault | L4 | 本地密码保险箱 | placeholder |
| mood_empathy / screen_guard | L5 | 心情识别增强 / 全屏守护 | active / placeholder |
| performance / dressup | L6 | 表演系统（节日彩蛋接口）/ 形象换装 | placeholder |
| story_line_1 / anniversary | L7 | 专属剧情线一（对话彩蛋模板）/ 纪念日自动庆祝 | placeholder / active |
| sleep_guard / voice_mode | L8 | 息屏守护 / 语音陪伴模式 | placeholder |
| story_line_2 | L9 | 专属剧情线二（双人彩蛋） | placeholder |
| finale / batch_skills | L10 | 缔结仪式结局演出 / 高级助理批量技能入口 | placeholder |

---

## 一·肆补、迭代①命令（全屏守护 + 代码检查，GUARD.md / CODECHECK.md 冻结契约）

> 冻结契约：[`GUARD.md`](GUARD.md)（全屏守护演出）、[`CODECHECK.md`](CODECHECK.md)（代码检查模块）。
> 守护调度状态机（`Idle → Active ⇄(finish/skip) Done →(30min 冷却) Idle`）与只读检查器全部收敛在 qibai-core；
> 浏览器 mock 层（`tauri-bridge.js`）与 Tauri 真机同名同形。

### 22. SettingsJson 扩展：新增 `guard` 块 + `skills` 增加 `check_code`

`SettingsJson`（§1）顶层新增 `guard` 块，`skills` 增加 `check_code` 键：

```jsonc
{
  "guard": {
    "enabled": true,            // 总开关；false 时任何触发返回 None
    "sit_min": 120,             // 连续工作多少分钟后触发（默认 120）
    "mode": "strong",           // light | strong | lock
    "end_action": "countdown",  // none | sleep | lock | countdown
    "countdown_secs": 120       // 休息倒计时秒数；反序列化后 clamp 到 60..=300
  },
  "skills": {
    // …既有 total_enabled / launch_app / open_folder / set_reminder…
    "check_code": true          // 代码检查技能（只读、无需审批、默认开）
  }
}
```

- `mode`：`light`（任何来源仅通知，`perform_fullscreen=false`，不打开守护窗）/ `strong`（全屏演出）/ `lock`（全屏演出且 `end_action` 强制改写为 `lock`）；
- `end_action`：`none`（直接结束）/ `sleep`（息屏）/ `lock`（锁屏）/ `countdown`（休息倒计时遮罩，60–300s）；
- 旧 settings.json（无 `guard` 块、`skills` 无 `check_code`）平滑加载，缺省取上述默认值（serde `default`，见 `settings.rs` 兼容单测 `legacy_settings_without_guard_or_check_code_loads`）。

### 23. 迭代①新增 invoke 命令

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `guard_trigger` | — | GuardStart payload（JSON）或 Err | 手动触发（桌宠右键「守护我」/ 久坐巡检透传）；未启用 / 演出中 / 30 分钟冷却内 → Err「守护当前不可用（未启用或冷却中）」；触发成功由外层创建/复用全屏 guard 窗（fullscreen + always_on_top + decorations=false）并广播 `qibai://guard-start`，payload 原样回给调用方 |
| `guard_skip` | `{reason: "user"\|"auto"}` | `"ok"` | 演出收尾（跳过不执行收尾动作）；关闭 guard 窗并广播 `qibai://guard-end {skipped, end_action}`；非演出态调用返回结构化错误 `guard is not active` |
| `guard_system_action` | `{action: "sleep"\|"lock"}` | `{simulated: bool, detail: string}` | 用 `platform_sleep_cmd()` / `platform_lock_cmd()` 按 `cfg(target_os)` 分发的命令 spawn 执行；沙箱 / 权限不足时 `simulated=true` 并在 detail 说明；执行结果广播 `qibai://guard-action` |
| `check_code` | `{path?: string, snippet?: string, language?: string}` | `CodeReport`（§25） | `path`（文件/目录）与 `snippet`（代码片段）恰好提供一个，否则结构化错误；只读扫描本地文件（单文件 ≤5MB、总计 ≤50MB、≤500 文件，自动跳过 `target/ node_modules/ .git/ _shots/ _backup/ .venv/ __pycache__/` 与隐藏目录） |

> **对话侧引擎级工具 `start_guard`**：不走技能白名单，由引擎工具表直接拦截（调 `manual_trigger`），用于伴侣在对话中主动发起守护演出；前端 tool schema 不可见、无需审批。

### 24. 迭代①新增事件（§二 扩展，编号续 §10）

| 事件 | payload 形状 | 时机 |
|---|---|---|
| `qibai://guard-start` | `{mode, hours, message, end_action, countdown_secs, perform_fullscreen, persona_id, screenshot?}` | 触发成功：创建/复用全屏 guard 窗并演出；`hours = max(1, sit_min/60)`；`screenshot` 为本地合成 data URL 铺底（缺失时前端 CSS 模拟桌面）；`perform_fullscreen=false`（light）时前端仅发通知不全屏 |
| `qibai://guard-end` | `{skipped: bool, end_action}` | 演出结束（自然 / 跳过），由 `guard_skip` 后广播 |
| `qibai://guard-action` | `{action: "sleep"\|"lock", simulated: bool, detail}` | `guard_system_action` 执行结果（成功 / 模拟降级） |

### 25. CodeReport 冻结形状（`check_code` 返回）

```jsonc
{
  "source": "path",                        // path | snippet
  "path": "/abs/path/to/dir-or-file",      // snippet 模式为 null
  "files_checked": 3,
  "language": "javascript",                // javascript/python/rust/typescript/json/html/css/markdown/text/unknown/mixed（目录模式 mixed）
  "summary": "发现 2 个问题：error 1 / warning 1 / hint 0",  // 无问题时为 "未发现问题"
  "issues": [
    {
      "severity": "error",                // error | warning | hint
      "category": "syntax",               // syntax | logic | security | style | unknown
      "message": "SyntaxError: Missing semicolon. (2:10)",
      "location": "/abs/path/to/file.js:2",  // "file:line" 字符串，或 null
      "suggestion": "根据报错行修正 JS 语法"    // 字符串，或 null
    }
  ],
  "llm_note": null                         // 无 key / LLM 失败时 null（纯静态报告是默认路径，不是错误）
}
```

> **门控备注**：亲密度解锁树 `screen_guard`（L5，§21 门控表）当前 kind 仍为 **placeholder**——全屏守护功能已实现并默认启用，**门控联动（L5 解锁门槛）待后续迭代切换启用（实现先行）**。

---

## 一·伍补、迭代②语音命令（TTS 发声陪伴，VOICE.md 交付文档）

> 交付文档：[`VOICE.md`](VOICE.md)（三平台命令表 / 音量语速映射 / 环境变量 / 云端端点与隐私边界 / STT 降级 / 真机验证清单）。
> 引擎选择 / 平台命令构造 / 云端请求全部收敛在 qibai-core；浏览器 mock 层（`tauri-bridge.js`）与 Tauri 真机同名同形。

### 26. SettingsJson 扩展：新增 `voice` 块

`SettingsJson`（§1）顶层新增 `voice` 块：

```jsonc
{
  "voice": {
    "enabled": true,            // 总开关（默认 true；无可用引擎时自动静音）
    "engine": "auto",           // auto | local | cloud（默认 auto）
    "volume": 80,               // 0-100，默认 80，落盘前 clamp
    "rate": 1.0,                // 0.5-2.0，默认 1.0，落盘前 clamp
    "scope": "all",             // all | reminders | chat（默认 all）
    "cloud_provider": "doubao",  // doubao | volcano（默认 doubao）
    "cloud_voice": "",          // 音色 ID，留空用引擎默认
    "cloud_api_key": "",        // 空则运行时读 QIBAI_TTS_KEY → DOUBAO_API_KEY；空字符串 skip_serializing_if
    "stt_enabled": false,       // STT 总开关（默认 false）
    "stt_provider": "doubao",
    "stt_api_key": ""           // 空则运行时读 QIBAI_STT_KEY → DOUBAO_API_KEY
  }
}
```

- 旧 `settings.json`（无 `voice` 块）平滑加载默认值（serde `default`，同 guard 先例）；前端 collect 必须整表带全 voice 字段，防止保存覆盖丢失。

### 27. 迭代②新增 invoke 命令

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `tts_speak` | `{text: string, kind: "chat"\|"reminder"\|"level_up"\|"guard"}` | `TtsSpeakResult`（见下） | 前端主动触发（对话回复朗读 / 桌宠喇叭重读）；`tts_decide` 为 None（总开关关 / scope 不覆盖 / 文本空白）→ `{engine:"mute", played:false, degraded:false, reason:"朗读未启用或范围未覆盖"}` |
| `tts_engine_status` | — | `{enabled, engine, volume, rate, scope, local_available, cloud_ready, resolved:{engine:"local"\|"cloud"\|"mute", reason}}` | 当前 TTS 引擎状态；`local_available` 由 `detect_local_tts_available()` 探测（沙箱无 espeak/say/powershell → false）；`cloud_ready` = key 可用 |
| `stt_status` | — | `{enabled, provider, key_ready, device_available, available, reason}` | STT 可用性；`available = enabled && key_ready && device_available`；`reason` 为中文提示（如「STT：未配置 STT Key（可通过环境变量 QIBAI_STT_KEY 提供）」），不可用时前端隐藏入口、不报错 |

`TtsSpeakResult` 形状：

```jsonc
{
  "engine": "local",                 // local | cloud | mute
  "played": true,                    // 本地 spawn 成功 true；云端合成成功 false（前端 <audio> 播 data URL）
  "degraded": false,                 // true = 云端失败后自动降级本地（或 Mute 兜底）
  "reason": "本地系统 TTS",          // mute / 降级时的中文说明
  "audio_data_url": "data:audio/wav;base64,……"  // 仅 engine=cloud 合成成功时存在
}
```

- resolved Local → `run_local_tts`（三平台命令见 `VOICE.md` §4）；
- resolved Cloud → `build_cloud_request` POST `https://openspeech.bytedance.com/api/v1/tts`（key 仅进 `X-Access-Token` header，绝不出现在 body）；合成失败自动降级：本地可用 → `{engine:"local", degraded:true}`，否则 `{engine:"mute", degraded:true, reason}`；
- 自动播报接线：提醒 tick（kind=reminder）、亲密度升级（kind=level_up，仅 all scope）、守护开场（kind=guard，仅 all scope）。

### 28. 朗读范围与降级规则（摘要，详见 VOICE.md §3）

| scope | chat | reminder | level_up | guard |
|---|---|---|---|---|
| `all` | ✅ | ✅ | ✅ | ✅ |
| `reminders` | ❌ | ✅ | ❌ | ❌ |
| `chat` | ✅ | ❌ | ❌ | ❌ |

降级链：总开关关 / scope 不覆盖 / 空文本 → Mute（不发声、不报错）；云端无 key → auto 降级本地、cloud 降级本地、双无 → Mute；云端合成失败 → 本地兜底 → Mute。

---

## 一·陆补、迭代③：本地密码保险箱命令（VAULT_SPEC.md 冻结契约）

> 冻结契约：[`VAULT_SPEC.md`](VAULT_SPEC.md)（§1 数据信封 / §2 设置项 / §3 状态机 / §4 命令 / §5 对话触发 / §7 安全边界）。
> 加密 / 状态机 / 门控逻辑全部收敛在 qibai-core（`vault.rs`）；Tauri 层只做钥匙串适配、OS 剪贴板写、文件写盘；浏览器 mock（`tauri-bridge.js` WebCrypto）与 Tauri 真机**同名同形**。本节与 `qibai-core/src/engine.rs` 保险箱门面（15 个方法）及 `src-tauri/src/main.rs` 命令注册逐条对齐。

### 29. SettingsJson 扩展：新增 `vault` 块

`SettingsJson`（§1）顶层新增 `vault` 块：

```jsonc
{
  "vault": {
    "idle_lock_min": 5,          // 闲置自动锁定（分钟，默认 5，可配 1..=120）
    "max_wrong_attempts": 5,     // 连续输错主密码次数（默认 5，仅 master_password 模式）
    "cooldown_secs": 30,         // 冷却秒数（默认 30）
    "clipboard_clear_secs": 20   // 剪贴板自动清除秒数（默认 20）
  }
}
```

- 旧 `settings.json`（无 `vault` 块）平滑加载默认值（serde `default`），不回退整体默认；`save_settings` 同步 `vault.update_settings`。

### 30. 迭代③新增 invoke 命令（15 个：14 个用户命令 + 1 个后端 tick）

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `vault_status` | —（Tauri 层探测 `keyring_available` 后传入） | `VaultStatus`（见 §31） | 仅读；L4 门控未达时 `gated=true` |
| `vault_setup` | `{master_password}` | `"ok"` | 仅 unset 且走主密码模式；生成盐 / verifier / 空库落盘 |
| `vault_setup_keyring` | — | `"ok"` | 仅 unset；Tauri 层生成随机 32B key 存系统钥匙串后交 core 落盘（keyring 模式） |
| `vault_unlock` | `{master_password}` | `"ok"` | 主密码解锁（防暴力计数 / 冷却） |
| `vault_unlock_system` | — | `"ok"` | keyring 模式：Tauri 层取 key → core 校验解锁（不计数失败） |
| `vault_lock` | — | `"ok"` | 立即手动锁定；广播 `qibai://vault-locked` |
| `vault_tick` | — | `bool`（本次是否触发了闲置锁） | **后端 15s tick 驱动，非前端主动调用**：闲置超时 → 自动锁 + 广播 `qibai://vault-locked` |
| `vault_list` | — | `[Entry]` | 仅 unlocked；自动 touch |
| `vault_search` | `{q}` | `[Entry]` | 本地过滤 site/account/note（大小写不敏感、中文子串）；仅 unlocked；自动 touch |
| `vault_add` | `{site, account, password, note, icon}` | `{id}` | 仅 unlocked；自动 touch |
| `vault_update` | `{id, site, account, password, note, icon}` | `"ok"` | 仅 unlocked；`password` 传空串表示不改；自动 touch |
| `vault_delete` | `{id}` | `"ok"` | 仅 unlocked；自动 touch |
| `vault_copy` | `{id}` | `{copied: true, clear_after_secs}` | core 返回明文给 Tauri/mock 层写剪贴板并调度清除；**返回给前端的载荷不含明文** |
| `vault_export` | — | `{path}` | core 生成 `.qbkv` payload；Tauri 层写 `<data_dir>/vault-backup-<ts>.qbkv`；mock 层 Blob 下载并返回占位 path |
| `vault_import_payload` | `{payload, master_password}` | `"ok"` | 用备份自身 kdf+密码解密校验；unset → 以备份重建；已初始化 unlocked → 本地 key 重加密合并（id 冲突覆盖）；locked → 结构化拒绝 `VaultLocked`。**导入成功后统一回到 locked**，随后按本地模式重新解锁 |

`Entry`：`{ id, site, account, password, note, icon, updated_at }`（列表展示密码一律前端掩码 `••••••••`）。

### 31. `VaultStatus` 返回形状

```jsonc
{
  "state": "unset|locked|unlocked",
  "mode": "none|keyring|master_password",
  "keyring_available": true,        // Tauri 层探测（mock 恒 true 表示"可用"路径演示）
  "gated": true,                    // active persona 亲密度 < 4 → 保险箱不可用
  "required_level": 4,
  "idle_lock_min": 5,
  "max_wrong_attempts": 5,
  "cooldown_secs": 30,
  "clipboard_clear_secs": 20,
  "failed_attempts": 0,
  "cooldown_remaining_secs": 0,
  "locked_reason": null,            // "manual"|"idle"|"cooldown"|null
  "entries_count": 0                // 仅 unlocked 时给出，否则 0
}
```

### 32. 错误码（结构化，不 panic）

| 错误 | 触发场景 |
|---|---|
| `VaultLocked` | 未解锁时调用写 / 读命令；locked 时导入备份 |
| `VaultCooldown{remaining_secs}` | 主密码模式连续输错 `max_wrong_attempts` 次后冷却期内的一切解锁尝试（含正确密码） |
| `VaultWrongPassword` | 主密码解锁密码错误（计数 +1） |
| `VaultNotInitialized` | unset 状态下调用解锁 / 列表等命令 |
| `VaultAlreadyUnlocked` | unlocked 状态下重复解锁 |
| `VaultGateLocked{required: 4}` | active persona 亲密度未达 L4 时调用任何写命令 |
| `VaultNotFound` | 对不存在的 id 执行 update / delete / copy |
| `VaultImportFailed` | 导入备份密码错误 / 信封损坏 |
| `VaultKeyringUnavailable` | keyring 模式操作但系统钥匙串不可用 |

### 33. 对话触发语义（engine.send_message 内、LLM 调用之前拦截）

- **触发判定（本地、无 LLM）**：文本含「密码」，且满足任一：含「保险箱」；匹配查询意图 `/密码.{0,6}(是什么|是多少|忘了|多少|帮我查|给我)/`；命中已解锁条目中任一 site/account 子串。
- **分支（按序）**：
  1. 亲密度未达 L4（`is_gate_open(level,"vault")==false`）→ 回复「亲密度达到 L4（贴心）后会解锁本地密码保险箱，继续和我聊天吧～」，`skill:"vault"`，不进 LLM；
  2. 命中且 `unset` → 回复引导「先在控制台·保险箱页初始化（设置主密码）」；
  3. 命中且 `locked` → 回复「保险箱已锁定，先在控制台解锁（主密码或系统解锁）后再问我吧」；
  4. 命中且 `unlocked` → 本地检索；找到 → `reply` 含明文（site/account/password）+ `secret:true` + `secret_entry_id`，前端自动 `vault_copy({id})`；未找到 → 「没找到和「XX」相关的密码记录哦」。
- **明文边界（冻结）**：`reply` 明文仅在当次返回给已解锁会话 UI；写入 conversation 的是脱敏文本；喇叭重读读 conversation，故明文永不发声；拦截路径绝不调用 LLM（单测用 mock provider 断言未被调用）。
- **`SendMessageOutput` 新增字段**（均 `#[serde(skip_serializing_if = "Option::is_none")]`）：`secret: Option<bool>` / `secret_entry_id: Option<String>` / `speak_text: Option<String>`（拦截回复 = 脱敏文本；非拦截 None，前端沿用 reply）。
- **剪贴板接缝**：core 拦截不做剪贴板，只返回 `secret_entry_id`；前端收到 `secret:true` 回复后调用 `vault_copy`（Tauri 层写系统剪贴板 + spawn 清除任务；mock 层 navigator.clipboard，失败降级）。语音朗读一律用 `speak_text`。

---

## 二、后端 emit 事件

### 1. `"qibai://reminder"` — 提醒触发
```jsonc
{
  "type": "sit",         // sit | water | eye
  "message": "已经坐了很久啦，站起来活动一下吧～",
  "action": "stretch"     // 建议前端播放的动作
}
```

### 2. `"qibai://llm-status"` — LLM 状态
```jsonc
{ "state": "thinking" }                        // 正在调用模型
{ "state": "done" }                            // 回复完成
{ "state": "error", "message": "missing API key: ..." }  // 出错
```

### 3. `"qibai://skill-approval"` — 危险操作待确认
```jsonc
{
  "request_id": "approval-1",
  "skill": "execute_command",
  "description": "即将执行命令：rm -rf /（来自对话技能请求）"
}
```
前端弹出确认 UI；用户选择后调用 `approve_skill({request_id, approve})`。

### 4. `"qibai://desk-changed"` — desk 在场名单变化
```jsonc
{ "present": ["gudian", "huopo"], "locked": { "gudian": false, "huopo": true } }
```
控制台：刷新角色管理/关系列表与客人选择器；桌宠窗：自己不在 present 时（仅 mock 场景）显示道别气泡。

### 5. `"qibai://pet-say"` — 指定桌宠窗气泡
```jsonc
{ "persona_id": "huopo", "text": "古典佳人，你也在这里呀！", "emotion": null, "action": null, "ms": 2200 }
```
仅 `persona_id` 匹配本窗的桌宠窗演出气泡 + 表情/动作；控制台忽略。

### 6. `"qibai://pk-round"` — 猜拳对局广播
```jsonc
{ "a": "gudian", "b": "huopo", "move_a": "scissors", "move_b": "rock",
  "result": "lose", "winner": "huopo",
  "lines": [ { "speaker": "huopo", "text": "耶！我赢啦！再来再来！" } ] }
```
双方桌宠窗：手势动画（本窗=自己的 move）+ 结果情绪；控制台：结果区显示双方手势与判定文案。

### 7. `"qibai://pk-end"` — 退出 PK 演出态
```jsonc
{ "a": "gudian", "b": "huopo" }
```
双方桌宠窗收起手势小卡、恢复默认表情。

### 8. `"qibai://toast"` — 轻提示
```jsonc
{ "text": "已保存", "persona_id": null }
```
控制台：toast；带 `persona_id` 时对应桌宠窗显示气泡。

### 9. `"qibai://console-open"` — 呼出控制台
```jsonc
{}
```
harness/托盘侧监听后显示控制台窗（真机由后端直接创建/ show 窗口）。

### 10. `"qibai://intimacy-updated"` — 亲密度变化（升级 / 解锁 / 成就）
```jsonc
{
  "persona_id": "gudian",
  "points": 720,
  "level": 6,
  "level_name": "亲近",
  "leveled_up": true,
  "unlocked": [{ "gate": "performance", "label": "表演系统", "required_level": 6, "kind": "placeholder" }],
  "achievements_granted": [{ "id": "level_3", "name": "心有默契", "ts": 1234567890 }]
}
```
- 每次 `intimacy_event` 成功变更后广播（含 `leveled_up=false` 的普通加分）；升级时额外写社会事件 `intimacy_level_up`（a=b=persona_id，detail 含 new_level/level_name/unlocked）；
- 消费方：控制台（升级动画 + banner + 门控重校）、桌宠窗（按 `persona_id` 过滤后播放庆祝动效 + 气泡）。

### 11. `"qibai://guard-start"` — 全屏守护演出开始（迭代①）
```jsonc
{
  "mode": "strong",                       // light | strong | lock
  "hours": 2,                             // max(1, sit_min/60)
  "message": "你连续工作 2 小时啦，休息五分钟吧",
  "end_action": "countdown",              // none | sleep | lock | countdown
  "countdown_secs": 120,                  // 已 clamp 到 60..=300
  "perform_fullscreen": true,             // light 恒 false（仅通知）
  "persona_id": "gudian",                 // 前端拼 assets/personas/<id>/character.png，失败回退 CSS 占位脸
  "screenshot": "data:image/png;base64,…"  // 可选：本地合成截图 data URL；缺失时前端 CSS 模拟桌面
}
```
- 触发成功后由外层创建/复用全屏 guard 窗（`guard.html`，fullscreen + always_on_top + decorations=false）并演出；
- `perform_fullscreen=false`（light）时前端只发通知、不打开全屏覆盖层；
- 演出流程：截图底图 → 图标碎裂特效（纯视觉）→ 伴侣现身 → 气泡劝休 → 收尾（none/sleep/lock/countdown）。

### 12. `"qibai://guard-end"` — 守护演出结束（迭代①）
```jsonc
{ "skipped": true, "end_action": "countdown" }
```
- 由 `guard_skip` 后广播；`skipped=true` 表示用户跳过（未执行收尾动作）。

### 13. `"qibai://guard-action"` — 息屏/锁屏执行结果（迭代①）
```jsonc
{ "action": "lock", "simulated": false, "detail": "已执行 loginctl lock-session" }
```
- `guard_system_action` 的执行结果；沙箱 / 权限不足时 `simulated=true`，detail 说明降级原因。

### 14. `"qibai://tts-speak"` — 朗读指示（迭代②）
```jsonc
{
  "kind": "chat",                 // chat | reminder | level_up | guard
  "engine": "local",              // local | cloud（Mute 不发本事件）
  "volume": 80,
  "rate": 1.0,
  "state": "speaking"
}
```
- 任何成功朗读路径（本地 spawn 或云端合成）后广播；Mute（总开关关 / scope 不覆盖 / 无可用引擎）**不发**本事件；
- 消费方：聊天页显示「正在朗读」指示；桌宠气泡加短暂 speaking 脉冲样式（动画结束后移除）。

### 15. `"qibai://vault-locked"` — 保险箱被锁定（迭代③）
```jsonc
{
  "reason": "manual",        // manual | idle | cooldown
  "state": "locked"
}
```
- 任何导致保险箱进入 locked 的路径后广播：手动 `vault_lock` / `vault_tick` 判定闲置超时 / 冷却触发 / 导入成功后回 locked；
- 消费方：控制台保险箱页刷新锁定横幅与解锁表单，隐藏条目列表。

---

## 三、枚举

### emotion（表情）
`smiling` · `happy` · `worried` · `sleepy` · `focused` · `sad` · `shy` · `surprised`

### action（动作）
`idle` · `wave` · `nod` · `shake` · `stretch` · `hands_on_hips` · `touch` · `celebrate`（节日彩蛋预留）

---

## 四、数据目录

| 平台 | 路径 |
|---|---|
| Linux | `~/.local/share/qibai/` |
| Windows | `%APPDATA%\qibai\` |
| macOS | `~/Library/Application Support/qibai/` |

文件：

- `settings.json`（设置）
- `conversation.json`（对话记录）
- `personas.json`（P0 角色实体：激活角色、昵称、性格参数、与用户亲密度、独立记忆，含 `sync` 远期联网预留字段）
- `social.json`（P1 社交：角色对好感度与交往等级、每日 PK/串门防刷计数、事件日志环形缓存）
- `intimacy.json`（亲密度 L1–L10：每角色 points/level、daily 每日计数、cumulative 累计、成就时间戳、纪念日、门控解锁状态、`migrated` 迁移标记、`sync` 远期联网预留字段——无文件时从 `personas.json` 的 `intimacy` 播种阈值起点积分，幂等）

隐私本地优先：全部本地存储，不上传。

---

## 一·柒补、迭代④：联网社交命令（星球 × 同城，NETSOCIAL_SPEC.md 冻结契约）

> 完整设计与冻结契约见 [`NETSOCIAL_SPEC.md`](NETSOCIAL_SPEC.md)。本节 17 个命令在 Tauri 真机（Rust core invoke）
> 与浏览器 mock 模式（`tauri-bridge.js` 直连真实本地服务端）中**同名同形**；全部失败路径返回结构化错误
> `EngineError::Net`（message 中文），不 panic。匿名优先：命令载荷中聊天数据只含 `peer_key`（sha256 前 12 hex），
> 不含原始 `anon_id` 明文；匿名凭证加密缓存于 `<data_dir>/netsocial.json`（AES-256-GCM）+ `netsocial.key`（权限 600）。

### 31. 状态机简述

```
Unregistered ──netsocial_register──▶ Registered（本地生成 anon_id UUIDv4）
Registered ──netsocial_connect(server_url)──▶ Online
Online ──netsocial_disconnect / 网络断──▶ Offline（保留配置，指数退避 1/2/4/8s 自动重连）
Registered/Offline ──netsocial_register(重设 profile)──▶ Registered
```

未注册调 connect/disconnect/业务命令 → `NotRegistered`/`NotOnline` 结构化错误；
connect 无 server_url 且无入参 → `missing_server_url`。

### 32. 迭代④新增 invoke 命令（17 个）

| 命令 | 参数 | 返回 | 说明 |
|---|---|---|---|
| `netsocial_status` | — | `{ state, anon_id, nickname, color_idx, city, server_url, online, registered, last_error }` | state ∈ unregistered/registered/online/offline |
| `netsocial_register` | `{ nickname?, color_idx?, soul_tags?, city?, avatar?, age_declared?, pure_mode? }` | `{ ok, anon_id, state, registered }` | 已有身份时仅更新 profile；nickname 空默认「星客」、color_idx clamp 0-11 |
| `netsocial_connect` | `{ server_url? }` | `{ ok, state }` | 无参用已存 server_url；缺参且无已存 → `missing_server_url` |
| `netsocial_disconnect` | — | `{ ok, state }` | Online → Offline，停止后台重连 |
| `netsocial_set_profile` | `{ nickname?, color_idx?, soul_tags?, city?, avatar? }` | `{ ok }` | 本地持久化 + 在线时 PUT /api/me |
| `netsocial_roam` | `{ tag?, city?, online?, q? }` | `{ souls: [...] }` | GET /api/souls，原样透传服务端 profile 字段（前端本地哈希化为 peer_key 展示） |
| `netsocial_match` | `{ limit? }` | `{ souls: [ { profile, score } ] }` | GET /api/match（§5 评分排序） |
| `netsocial_city_souls` | `{ city }` | `{ souls, city }` | GET /api/city/:city/souls（同城灯塔） |
| `netsocial_like` | `{ id }` | `{ liked, matched, mutual }` | 点亮；matched=true 时前端提示「互相点亮」 |
| `netsocial_report` | `{ id, reason }` | `{ ok }` | 举报（不阻断通信，仅记录） |
| `netsocial_block` | `{ id }` | `{ ok, blocked }` | 拉黑（双向屏蔽） |
| `netsocial_send` | `{ id, text }` | `{ ok, mid? }` | WS msg.send；离线/未连接 → `not_online`；text 空/超 500 字符拒绝 |
| `netsocial_history` | `{ id? }` | `{ contacts: [...], messages: [...] }` | 按 peer_key 过滤或全部；消息形状 `{ peer_key, text, ts, dir }`（dir=in/out） |
| `netsocial_poll` | — | `{ events: [ { type, payload } ... ] }` | 消费后清空；无事件返回空数组 |
| `netsocial_topics` | — | `{ topics: [...] }` | 话题墙占位（标注「即将上线」） |
| `netsocial_events` | — | `{ events: [...] }` | 活动预告占位（标注「即将上线」） |
| `netsocial_cache_clear` | — | `{ ok }` | 删除 netsocial.json + netsocial.key，重置为 Unregistered |

### 33. 事件轮询契约（`netsocial_poll`，前端 2s 轮询）

事件统一形状 `{ "type": <类型>, "payload": {...} }`，环形队列上限 200（溢出丢最旧），poll 后清空：

| type | payload | 前端动作 |
|---|---|---|
| `connected` | `{ server_url }` | 连接状态条置「已连接」 |
| `disconnected` | `{ reason }` | 状态条置「离线」；自动重连中 |
| `hello` | `{ profile }` | 更新我的光点信息 |
| `inbox` | `{ count }` | 离线补投消息数提示 |
| `msg` | `{ peer_key, peer_nick, peer_color, text, ts, dir }` | 聊天窗口追加气泡（只显示昵称+光点色） |
| `presence` | `{ peer_key, online }` | 更新联系人在线点 |
| `error` | `{ code, message }` | toast 提示（如 blocked/censored/invalid_token） |

### 34. 技能扩展（send_message 回填）

- 白名单新增 `open_social`（只读安全技能，默认启用、无需审批）；`get_skills_status` 的 `skills` 块新增同名字段。
- LLM tool schema 新增 `open_social`（参数 `{ view?: "planet"|"city" }`；触发意图：带我去星球逛逛/星球广场/去星球看看/同城看看/打开社交）。
- `send_message` 回复 JSON 的 `skill` 字段命中 `open_social` 时，前端打开「社交」tab 并切换到对应视图。

### 35. 数据目录新增文件

- `netsocial.json`（AES-256-GCM 加密信封：anon_id/token/profile/脱敏联系人/脱敏消息）
- `netsocial.key`（随机 32B 密钥，权限 600；生产建议后续升级为 vault 主口令加密）
