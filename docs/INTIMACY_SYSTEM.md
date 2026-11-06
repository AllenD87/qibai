# 元气小小 Qibai — 亲密度成长系统（L1–L10）冻结设计规格

> 本文档是本次迭代的**唯一对齐基准**：所有实现（qibai-core / tauri-bridge / 控制台 UI / 桌宠 UI）
> 必须严格按下述冻结内容落地，任何人不得自行修改数值、门控键或契约形状。
> 如有冲突以本文档为准；确需调整须回到主控确认并更新本文档后再改代码。
> 冻结日期：2026-10-22。范围：v5 迭代（亲密度 L1-L10 成长系统）。

---

## 1. 总览

- 每位角色（persona）**独立**亲密度：独立积分、独立等级、独立解锁状态、独立成就、独立纪念日。
- 与既有"角色间好感度 / 交往系统"（social.json）**并行不冲突**：两者数据完全分离。
- 门控统一由 qibai-core 判定（`check_gate`），前端只读状态、不自行实现等级逻辑。
- 本地优先隐私：亲密度数据只落本地 `intimacy.json`，不上传；预留 `sync` 字段只写不读。
- 硬约束：合规全龄无擦边；不新增美术资产，动效全部用 CSS/JS。

---

## 2. L1–L10 等级表（冻结，不许改）

累计积分阈值（points 单调累积，跨天不清零）：

| 等级 | 名称 | 阈值起点 | 描述（UI 展示） |
|---|---|---|---|
| 1 | 初识 | 0 | 初见倾心，从一句你好开始。 |
| 2 | 熟悉 | 120 | 开始习惯彼此的存在。 |
| 3 | 习惯 | 280 | 你的作息与喜好，她悄悄记下。 |
| 4 | 贴心 | 480 | 她开始帮你打理生活小事。 |
| 5 | 默契 | 720 | 一个眼神，她就懂你的心情。 |
| 6 | 亲近 | 1000 | 想为你准备更多惊喜。 |
| 7 | 羁绊 | 1330 | 值得纪念的日子，她不会忘。 |
| 8 | 依赖 | 1720 | 她总想多陪你一会儿。 |
| 9 | 灵魂 | 2180 | 你们之间有了只有彼此懂的默契。 |
| 10 | 缔结 | 2720 | 独一无二的约定，只属于你们。 |

- `level_of(points) -> u8`：按上述阈值取最大满足档（1–10）。
- 升级判定：`intimacy_event` 加积分后重算等级，新等级 > 旧等级即升级。

---

## 3. 成长来源与每日上限（冻结）

### 3.1 积分来源

| source 键 | 触发 | 积分 | 防刷细则 |
|---|---|---|---|
| `chat` | 每次成功对话往返（用户发消息且收到回复后上报一次） | +2 | 计入每日总上限 |
| `click` | 桌宠窗点按/抚摸角色一次 | +1 | 每角色每日 click 上限 10 次，超出不再加分 |
| `task` | 完成每日任务（detail.task_id） | +5/个 | 每任务每日仅首次完成加分 |
| `companion` | 桌宠连续陪伴（pet 窗每 10 分钟上报一次） | 见 3.2 | 每角色每日 companion 积分上限 9 |
| `gift` | 节日礼物（detail 可带 label） | +10 | 每角色每日仅首次 gift 加分（记录 last_gift_date） |

### 3.2 连续陪伴加成递减（冻结公式）

companion 上报按**累计陪伴分钟**结算，加成频率随连续时长递减：

| 累计分钟达到 | 本次加成 | 间隔 |
|---|---|---|
| 10 / 20 / 30 | 各 +1 | 每 10 分钟 |
| 50 / 70 / 90 | 各 +1 | 每 20 分钟 |
| 120 / 150 / 180 | 各 +1 | 每 30 分钟 |

- 每角色每日 companion 积分上限 **9**（180 分钟即打满，之后不再加）。
- 实现：entry 存 `companion_minutes`（当日累计）与 `companion_points`（当日已得）；
  上报时 minutes += 10（detail.minutes 可覆盖，测试用），结算跨过的所有阈值。
- 注：阈值表共 9 档（10…180），打满即 9 分；"上限 10"系早期草稿笔误，已勘正为 9，以本表为准。

### 3.3 每日总上限（防刷核心）

- 每角色每日总积分获取上限 **60**（`daily.gained` 当日已得，`daily.date` 跨天自动重置）。
- 任一来源的增量在加之前先检查：`min(delta, 60 - daily.gained)`，为 0 则不加、事件结果标记 `capped: true`。
- 上限只限制"获取"，不扣已有积分。

### 3.4 每日任务（冻结 3 个）

| task_id | 名称 | 完成条件 | 积分 |
|---|---|---|---|
| `greeting` | 和她说早安 | 用户当日首次发消息且文本匹配 `早安|早上好|早呀|早啊|morning`（前端判定，调 `intimacy_event({kind:'task', detail:{task_id:'greeting'}})`） | +5 |
| `click` | 完成一次互动 | 当日首次 click 事件自动完成 | +5 |
| `pk` | 赢一场猜拳 | 当日首次猜拳获胜自动完成（前端在 PK 结果胜时上报） | +5 |

- 每任务每日仅首次完成加分；完成后 `tasks.{id}=true`，跨天重置。
- task 事件完成且未加分过 → 加 5 分（同时计入每日总上限）。

---

## 4. 解锁树与门控（冻结）

门控键全集（`gate` → 所需等级 → UI 标签 → 类型）：

| gate | 等级 | UI 标签 | 类型 |
|---|---|---|---|
| `basic_chat` | 1 | 基础对话 | active |
| `idle_actions` | 1 | 待机动作 | active |
| `sit_reminder` | 1 | 久坐提醒 | active |
| `custom_nickname` | 2 | 自定义称呼 | active |
| `daily_greeting` | 2 | 每日问候 | active |
| `water_reminder` | 2 | 喝水提醒 | active |
| `preference_memory` | 3 | 记住偏好（作息/常用软件） | active |
| `boot_greeting` | 3 | 开机问候 | active |
| `assistant_skills` | 4 | 助理技能（打开应用/文件夹、设提醒） | active |
| `vault` | 4 | 本地密码保险箱 | **placeholder**（仅解锁入口+占位，本体下一迭代） |
| `mood_empathy` | 5 | 心情识别增强 | active |
| `screen_guard` | 5 | 全屏守护 | **placeholder**（仅入口+占位提示） |
| `performance` | 6 | 表演系统（节日彩蛋接口） | active |
| `dressup` | 6 | 形象换装 | **placeholder**（皮肤切换占位） |
| `storyline_1` | 7 | 专属剧情线一（对话彩蛋模板） | active |
| `anniversary` | 7 | 纪念日自动庆祝（纪念日设置） | active |
| `sleep_guard` | 8 | 息屏守护 | **placeholder**（仅入口） |
| `voice_mode` | 8 | 语音陪伴模式 | **placeholder**（语音本体后续） |
| `storyline_2` | 9 | 专属剧情线二（双人彩蛋） | active |
| `ceremony` | 10 | 缔结仪式结局演出（文字+CSS 动效） | active |
| `batch_skills` | 10 | 高级助理批量技能 | **placeholder**（入口占位） |

- `gate_required(gate) -> u8`（上表）；`is_gate_open(level, gate) = level >= gate_required(gate)`。
- **真实门控示例（验收断言）**：`assistant_skills` 需 L4。L1 时 `check_gate` 返回 `open=false`；
  积分到 480（L4）后 `open=true`。引擎层技能执行路径（approve_skill / 技能执行）对 active persona
  校验 `assistant_skills` 门控，未开则拒绝并返回明确错误。
- placeholder 类型仅影响 UI 展示（"即将解锁"样式），不影响门控判定。

---

## 5. 成就（冻结 9 个）

| id | 名称 | 条件 |
|---|---|---|
| `first_greeting` | 初次问候 | 完成过 greeting 任务（cumulative.greetings ≥ 1） |
| `chat_100` | 聊得来 | cumulative.chats ≥ 100 |
| `click_50` | 指尖温情 | cumulative.clicks ≥ 50 |
| `pk_win_5` | 猜拳小能手 | cumulative.pk_wins ≥ 5 |
| `companion_7` | 常伴七日 | cumulative.companion_days ≥ 7 |
| `level_3` | 心有默契 | 等级 ≥ 3 |
| `level_6` | 亲密无间 | 等级 ≥ 6 |
| `level_9` | 灵魂共鸣 | 等级 ≥ 9 |
| `level_10` | 缔结之约 | 等级 = 10 |

- 判定时机：每次 `intimacy_event` 后统一重算；新达成者追加 `achievements[{id, ts}]` 并出现在返回的 `achievements_granted` 与广播里。
- `cumulative` 各计数：chats（chat 事件累计）、clicks、pk_wins（前端在 PK 胜时上报 `intimacy_event({kind:'chat'...})`？**否**——单独上报 `intimacy_event({kind:'pk_win'})`，+0 积分仅记成就）、companion_days（当日 companion 积分 >0 记一天）、greetings（greeting 任务完成累计）。

> 注：`pk_win` 为**新 source 键**：积分 +0，只累计 `cumulative.pk_wins`（供成就与任务）。归入 source 表一并实现。

---

## 6. IPC 契约（冻结 JSON 形状）

### 6.1 命令（engine 方法 / main.rs 注册 / bridge mock 同名实现）

1. `get_intimacy_levels()` → `{ levels: [{ level, name, desc, min, max }] }`（10 项，max=null 表示 ∞）
2. `get_intimacy_state({ persona_id })` →
```json
{
  "persona_id": "gudian",
  "points": 123,
  "level": 2,
  "level_name": "熟悉",
  "progress": { "min": 120, "max": 280, "ratio": 0.01875 },
  "unlocked": ["basic_chat", "idle_actions", "sit_reminder", "custom_nickname", "daily_greeting", "water_reminder"],
  "locked": [ { "gate": "assistant_skills", "label": "助理技能（打开应用/文件夹、设提醒）", "required_level": 4, "kind": "active" } ],
  "daily": { "date": "2026-10-22", "gained": 12, "cap": 60 },
  "cumulative": { "chats": 5, "clicks": 2, "pk_wins": 0, "companion_days": 1, "greetings": 0, "gifts": 0 },
  "tasks": [ { "id": "greeting", "name": "和她说早安", "done": false }, { "id": "click", "name": "完成一次互动", "done": false }, { "id": "pk", "name": "赢一场猜拳", "done": false } ],
  "achievements": [ { "id": "first_greeting", "name": "初次问候", "ts": 0 } ],
  "anniversary": null,
  "sync": { "server_id": null, "dirty": false, "version": 0 }
}
```
3. `intimacy_event({ persona_id, kind, detail? })`（kind ∈ `chat|click|task|companion|gift|pk_win`；task 需要 `detail.task_id`；companion 可选 `detail.minutes`（默认 10，测试用））→
```json
{
  "delta": 2, "points": 125, "level": 2, "leveled_up": false,
  "unlocked": [ { "gate": "xxx", "label": "...", "required_level": 6, "kind": "active" } ],
  "achievements_granted": [ { "id": "chat_100", "name": "聊得来", "ts": 1750000000 } ],
  "tasks_updated": [ { "id": "click", "done": true } ],
  "capped": false
}
```
   - 升级时：写事件日志（social events，kind=`intimacy_level_up`，a=persona_id，b=persona_id，detail 含 new_level/level_name/unlocked），并在 `unlocked` 中给出**本次新解锁**的门控（含 placeholder）。
4. `set_anniversary({ persona_id, month, day })` → `"ok"`；`clear_anniversary({ persona_id })` → `"ok"`（month 1-12、day 1-31 校验，非法报错）
5. `check_gate({ persona_id, gate })` → `{ "open": true, "required_level": 4, "current_level": 5 }`
6. `get_intimacy_state` 未知 persona → 报错；但 `check_gate` 对未知 persona 返回 current_level=1 不报错（宽容）。

### 6.2 事件（广播）

- `qibai://intimacy-updated` payload：
```json
{ "persona_id": "gudian", "points": 125, "level": 2, "level_name": "熟悉",
  "leveled_up": false, "unlocked": [], "achievements_granted": [] }
```
- 每次 `intimacy_event` 成功后由后端 app-wide emit（main.rs），bridge mock 用 `broadcast()` 保持同形状。
- 控制台与桌宠窗都订阅：控制台刷新亲密度页；桌宠窗在 `leveled_up=true` 时播庆祝动效。

### 6.3 门控只读原则

- 前端任何"是否可用"判定只通过 `check_gate`（或 `get_intimacy_state().unlocked/locked`）读取；
- 前端不得复制等级表/阈值做判断；等级表仅由 `get_intimacy_levels` 提供展示。

---

## 7. 存储与迁移（冻结）

文件：`<data_dir>/intimacy.json`

```json
{
  "version": 1,
  "migrated": true,
  "entries": {
    "gudian": {
      "persona_id": "gudian",
      "points": 1330,
      "level": 7,
      "daily": { "date": "2026-10-22", "gained": 0, "chats": 0, "clicks": 0, "companion_minutes": 0, "companion_points": 0, "tasks": { "greeting": false, "click": false, "pk": false } },
      "cumulative": { "chats": 0, "clicks": 0, "pk_wins": 0, "companion_days": 0, "greetings": 0, "gifts": 0, "last_gift_date": null },
      "achievements": [],
      "anniversary": null,
      "sync": { "server_id": null, "dirty": false, "version": 0 }
    }
  }
}
```

- `entries` 用 BTreeMap/有序 map，序列化键序稳定。
- **迁移（幂等）**：
  1. 文件不存在 → 首次初始化 5 角色实体（按 personas 表），`migrated=false`，落盘。
  2. `migrated=false` 时：对每个角色，读取 `personas.json` 中该角色的 `intimacy`（1–10，旧 settings.intimacy_level 已由 P0 迁入 personas.json），
     seed points = 该等级阈值起点（如 intimacy=7 → points=1330）；置 `migrated=true` 落盘。
  3. 再次加载：`migrated=true` 不再播种；无 personas.json 时按 intimacy=1（points=0）处理。
  4. 缺字段的旧文件（无 daily/cumulative/achievements 等）→ serde default 补齐，不报错不丢数据。
- 测试断言：迁移幂等（连跑 3 次结果一致）；personas.json intimacy=7 → points=1330, level=7。

---

## 8. 前端行为契约

### 8.1 控制台「亲密度」tab（console）

- Tab 名 `亲密度`，位置放在「角色」之后。
- 顶部：当前角色选择 chips（5 角色，默认 active persona，切换即重新拉取）。
- 等级区：等级徽章（L1-L10 数字+名称）、一句话描述、进度条（当前档内 points/min..max，带百分比）、积分数字、今日已得/上限。
- 解锁列表：按等级分组展示 21 个门控，已解锁 ✓（active 绿色 / placeholder 黄色"已解锁·占位"），未解锁 🔒（显示所需等级）。
- 成就区：9 个成就徽章网格（已达成彩色 / 未达成灰）。
- 每日任务面板：3 任务卡片（done 勾选状态），可点「补领」？**不**——任务自动完成，面板只读展示；greeting 任务另提供「说早安」快捷按钮（点了即上报 task greeting，等同发送含早安消息）。
- 纪念日设置行：月/日 输入 + 保存/清除（L7 门控未开时显示"L7 解锁"禁用）。
- 升级解锁通知：页面内 banner/toast——当 `intimacy-updated.leveled_up` 或本地事件返回 `unlocked` 非空时，展示"🎉 升级至 Lx「名称」+ 新解锁 xx"。
- 升级动画：等级徽章放大+光晕+飘星 CSS 动画（复用可触发函数）。
- 技能页联动：设置 tab「技能授权」区块——`check_gate('assistant_skills')` 为 false 时整区块显示锁定横幅"助理技能需亲密度 L4 解锁"且开关禁用；为 true 恢复。
- 聊天联动：发送消息收到回复后调用 `intimacy_event({kind:'chat'})`；发送文本命中早安正则时补报 `intimacy_event({kind:'task', detail:{task_id:'greeting'}})`。

### 8.2 桌宠窗（pet）

- 点按角色（stage/characterWrap 单击，排除拖拽与右键）→ `intimacy_event({kind:'click'})` + 爱心/星星飘浮 CSS 动效 + 随机一句撒娇气泡（沿用 PET_LINES 池）。
- `qibai://intimacy-updated` 且 `leveled_up=true` → 全窗庆祝动效（光晕+飘心+跳跃+升级气泡"我们更亲近啦，现在是 Lx「名称」！"），气泡内附新解锁 1-2 条。
- 陪伴上报：每 10 分钟 `intimacy_event({kind:'companion'})`（浏览器 mock 用缩短间隔仅 dev 面板调试，正式 10 分钟）。
- dev 面板（仅浏览器模式）：新增「+10 分」「升级演示」「点按互动」按钮，供冒烟截图。

### 8.3 bridge mock（浏览器模式行为等价）

- 新增 `LS_KEYS.intimacy`；mock 存储与 §7 同构；无数据时从 mock personas（`LS_KEYS.personas`）播种迁移（等价规则）。
- 命令形状、事件广播与 §6 完全一致；`qibai://intimacy-updated` 用 `broadcast()`。
- `__qibaiMock` 增加 `intimacy: { event(kind, detail), seed(level), reset() }` 供冒烟。

---

## 9. 单测清单（qibai-core 新增，验收映射）

1. `level_of` 边界：0→1、119→1、120→2、279→2、280→3、479→3、480→4、719→4、720→5、999→5、1000→6、1329→6、1330→7、1719→7、1720→8、2179→8、2180→9、2719→9、2720→10。
2. chat 事件：+2；daily.gained 累计。
3. click 事件：+1；当日第 11 次 click 不再加分（delta=0）。
4. 每日总上限：已得 59 时 chat 只加 1（capped=true）；已得 60 时 delta=0。
5. companion 递减：minutes 10→+1、30→累计 3、40 不触发、50→+1；当日 companion_points 封顶 **9**（到 180 分钟打满）。
6. task 完成：greeting/click/pk 各 +5 一次；同日重复任务事件 delta=0；tasks 跨天重置。
7. gift：+10；同日第二次 gift delta=0（last_gift_date）。
8. 升级：points 120→等级 2，`leveled_up=true`，`unlocked` 含新门控，事件日志写 `intimacy_level_up`（含 new_level）。
9. 门控断言（验收示例）：L1 时 `is_gate_open(1,'assistant_skills')==false`；L4（points≥480）后 true；`gate_required` 全表核对 21 键。
10. 成就：chats≥100 → chat_100 达成；等级 10 → level_10；companion_days≥7 → companion_7。
11. 迁移幂等：personas.json intimacy=7 → points=1330/level=7；连载 3 次不变；缺字段旧文件 default 补齐。
12. 每日计数跨天重置：把 daily.date 置为旧日期再事件，daily 全部归零。
13. `check_gate` 未知 persona 宽容返回。
14. 技能门控：engine approve_skill 在 L1 时报"亲密度不足"类错误，L4 后放行（或等价断言）。

**验收声明**：`cargo test -p qibai-core` 全部通过（既有 58 + 新增 ≥14）。

---

## 10. 冒烟验收（截图清单）

| 截图 | 内容 |
|---|---|
| `intimacy-page.png` | 控制台亲密度页全景：等级徽章/进度/解锁列表/成就/每日任务 |
| `intimacy-upgrade-anim.png` | 亲密度页升级动画瞬间（徽章放大光晕） |
| `intimacy-unlock-notify.png` | 升级解锁通知 banner（新解锁列表） |
| `pet-celebrate.png` | 桌宠窗升级庆祝动效（飘心+气泡） |
| `intimacy-gate-lock.png` | 设置页技能区 L1 锁定横幅 + 亲密度页锁上门控展示 |

另：回归截图 1-2 张（聊天/事件页正常）。

---

## 11. 受限项（如实标注）

- src-tauri 在本沙箱不编译（缺 gdk-3.0/webkit2gtk-4.1 等系统库，系统目录只读无法安装）：
  main.rs 新命令仅代码接线，真机行为待装齐依赖验证（沿用既有受限声明）。
- 升级/庆祝动效截图以浏览器 mock 驱动（`__qibaiMock`/dev 面板），Tauri 真机表现待验证。
- 节日礼物无真实节日日历联动（本迭代按事件上报计分，日历联动后续迭代）。
