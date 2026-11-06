# 《元气小小》P0+P1 多角色社交扩展 — 实施规划（冻结契约）

> 状态：**已冻结**。本文件是 P0（角色实体化）+ P1（本地多角色社交闭环）的接口契约与实施规划。
> 后端（qibai-core）与前端（src/）两个并行工作包必须严格按本契约实现，命令名 / 参数字段 / 返回 JSON 形状以本文件为准。
> 联网版（跨用户串门 / 账号 / 服务端）列入远期，本次不实现，仅在数据层预留 `sync` 字段。

---

## 一、范围与目标

1. **P0 角色实体化**：5 个形象（`gudian/dushi/wenrou/huopo/reqing`）从"皮肤"升级为独立角色实体：
   `persona_id`、独立性格参数、独立记忆（对话片段）、独立亲密度（与用户）、角色间好感度；
   数据模型放本地 JSON，预留服务端同步字段，迁移不破坏旧数据。
2. **P1 双角色同屏与互动**：召唤第二个角色上桌（双角色可分别拖动）→ 串门（走入 + 寒暄）→
   PK（猜拳，每日每对 3 次防刷，胜方好感 +、败方安慰彩蛋）→ 交往（4 级关系：初识→朋友→闺蜜/搭档→羁绊，称谓变化 + 等级解锁彩蛋）。
3. **配套**：事件日志（可回看）；设置页角色管理面板（召唤/送走、关系与好感度、事件日志入口）。
4. **回归与交付**：qibai-core 测试保持全过（既有 32/32 全过 + 新增单测）；README/CHECKLIST 更新；打包 zip（排除 `qibai/target`）；冒烟截图（召唤→串门→猜拳→好感度变化→日志）。

## 二、设计决策（冻结）

| # | 决策 | 理由 |
|---|---|---|
| D1 | 存储用 **JSON 文件族**（`personas.json` + `social.json`），不引入 SQLite | 兼容现有 settings.json/conversation.json 模式；纯库不新增依赖 |
| D2 | **谁在桌上**（召唤状态）为纯前端状态（`localStorage('qibai.desk')`），core 只管社交数据与规则 | 职责清晰；浏览器 mock 可完整冒烟 |
| D3 | 寒暄台词 **模板优先**（内置剧本表，按 persona 对 × 关系等级分档），配 key 时可选 LLM 轻量生成（单轮、低 token），无 key 自动降级模板 | 控制 token 成本；离线可用 |
| D4 | PK **出拳在前端、判定/计分/防刷在 core** | 保证好感度变化可信且防本地刷 |
| D5 | 关系等级 4 级，affinity 阈值与称谓见 §三；PK/串门计分规则见 §三 | 数值全部收敛在 core，前端只展示 |
| D6 | **不新增美术**：双人演出用现有 5 形象资产 + CSS 动效 | 硬约束（沿用长期惯例） |
| D7 | **不改 src-tauri**（沙箱缺系统库不可编译验证）：窗口仍 220×260，双人布局窗口内自适应缩放；本期召唤入口在**设置页 + 主窗快捷按钮**，托盘菜单项列入 README 待真机项 | 保证本次改动全部可验证 |
| D8 | 合规：LLM 双人对话 system prompt 强制全龄、无擦边、角色间为友好社交关系；文案全中文 | 硬约束 |

## 三、数据模型（冻结）

### 3.1 `personas.json`（新文件，core 负责读写）
```jsonc
{
  "version": 1,
  "active_persona": "gudian",
  "personas": [
    {
      "persona_id": "gudian",          // 枚举：gudian/dushi/wenrou/huopo/reqing
      "display_name": "古典佳人",       // 现有注册表显示名
      "nickname": "元气小小",               // 用户可重命名
      "traits": {                      // 独立性格参数（persona.rs 性格描述的结构化摘要）
        "tone": "温婉", "catchphrase": "……", "interaction": "……"
      },
      "intimacy": 1,                   // 与用户的亲密度 1-10（迁移自旧 settings.intimacy_level）
      "memory": [ {"role": "user", "content": "…", "ts": 0} ],  // 独立记忆，上限 50 条环形裁剪
      "sync": { "server_id": null, "dirty": false, "version": 0 } // 远期联网预留，本期只写不读
    }
  ]
}
```

### 3.2 `social.json`（新文件，core 负责读写）
```jsonc
{
  "version": 1,
  "relations": [
    {
      "a": "gudian", "b": "huopo",        // 无序对，a<b 字典序归一
      "affinity": 0,                      // 角色间好感度
      "level": 0,                         // 0-3，见等级表
      "title": "初识",
      "pk_count": { "date": "2026-10-21", "count": 0 },  // 每日每对 PK 次数（防刷）
      "greet_today": 0, "greet_date": "2026-10-21",       // 每日串门计分次数（上限 5）
      "summon_count": 0, "greet_count": 0, "pk_total": 0,
      "last_interaction_ts": 0
    }
  ],
  "events": [                            // 事件日志，上限 200 条环形裁剪
    {
      "ts": 1730000000, "type": "greet|pk|level_up|summon|dismiss",
      "a": "gudian", "b": "huopo",
      "detail": { "winner": "huopo", "affinity_delta": 10, "new_level": 1, "text": "……" }
    }
  ]
}
```

### 3.3 关系等级表（冻结）
| level | 名称 | affinity 区间 | 称谓示例 |
|---|---|---|---|
| 0 | 初识 | 0 – 99 | 互称全名（"古典佳人"） |
| 1 | 朋友 | 100 – 299 | 昵称（"佳人"） |
| 2 | 闺蜜 / 搭档 | 300 – 599 | 专属昵称（"佳佳"） |
| 3 | 羁绊 | 600+ | 挚友 / 搭档宣言 |

### 3.4 计分规则（冻结，全部在 core 内实现）
- PK 胜利方 affinity **+10**，失败方 **+2**（安慰），平局双方 **+4**；
- 串门寒暄计分：每次 **+1**，每日每对上限 **5 次**（防刷）；
- 升级（level 跨档）写入 `level_up` 事件并返回解锁彩蛋台词；
- PK 防刷：`pk_count.date` 为今天且 `count >= 3` 时拒绝，返回 `remaining_today: 0` 与提示文案。

## 四、IPC 契约扩展（冻结，core 命令名 = tauri-bridge mock 命令名）

| 命令 | 参数 | 返回 |
|---|---|---|
| `get_personas` | – | `{ active_persona, personas: [...] }`（personas 数组每项含 persona_id/display_name/nickname/traits/intimacy/memory 最近 N 条/sync） |
| `set_active_persona` | `{ persona_id }` | `"ok"` |
| `rename_persona` | `{ persona_id, nickname }` | `"ok"` |
| `get_social_state` | – | `{ relations: [...], levels: [ {level,name,title,min,max} x4 ] }` |
| `social_greet` | `{ from, to, use_llm? }` | `{ lines: [{speaker, text}], affinity_delta, event }`（无 key 时忽略 use_llm 走模板） |
| `social_pk` | `{ a, b, move_a, move_b }` | `{ winner, loser, result, affinity_delta, lines: [...], remaining_today, blocked: bool }`；blocked=true 时其余字段可为空、lines 含提示文案 |
| `get_social_events` | `{ limit? }` | `{ events: [...] }` |

- `move` 枚举（前端中文显示"石头/剪刀/布"，传输用英文 key）：`rock | paper | scissors`；
- `lines` 中 `speaker` 为该轮台词所属 `persona_id`，前端据此决定哪个角色气泡发声；
- 事件日志 `type ∈ { greet, pk, level_up, summon, dismiss }`；
- 所有新命令失败路径返回结构化错误（如未知 persona_id / 未知对 / 今日次数用尽），不 panic。

## 五、前端结构（冻结）

- `src/index.html`：`#stage` 内新增第二个角色容器（`#guestWrap`，结构对齐现有 `#characterWrap`）；底部工具条在桌上有客角色时显示「串门」「PK」快捷按钮；
- `src/app.js`：双角色渲染（复用现有 `PERSONAS` 注册表）、窗口内拖动（pointer events，避开 `data-tauri-drag-region`）、串门走入动画（CSS transform + transition，从窗口边缘走入 A 身边）、寒暄气泡演出（双气泡按 speaker 交替）、猜拳 UI（双方出拳动画 → 判定 → 结果演出 → 好感度/等级 toast）、剩余次数提示；
- `src/settings.html` / `src/settings.js`：**角色管理面板**（5 角色卡片：当前激活 / 召唤上桌 / 送走 / 重命名昵称）、**关系面板**（每对 affinity 进度条 + 等级 + 称谓 + 今日 PK 次数）、**事件日志面板**（时间倒序列表回看）；
- `src/tauri-bridge.js`：`mockInvoke` 增加上述新命令的 localStorage 假实现（`qibai.mock.personas` / `qibai.mock.social`），Tauri 分支透传同名命令；
- `src/styles.css`：沿用现有色板（主 `#FBD3E4`/`#D17397` 系），新增双人布局（220×260 窗口内自适应缩放）、出拳 UI、进度条、事件列表样式；
- 冒烟 hash 钩子（浏览器 mock 模式驱动）：`#social=summon&b=huopo`（召唤+双人同屏）、`#social=greet`（串门寒暄）、`#social=pk`（猜拳+结果+好感度变化）、`#social=events`（事件日志）、`#social=settings`（设置页角色管理）；
- 既有 hash 钩子（`#emotion=` / `#persona=` 等）不得破坏。

## 六、验收（冻结）

- [ ] 5 角色可独立召唤/送走；双角色同屏、分别可拖动（前端冒烟截图证明）；
- [ ] 串门→寒暄（双气泡演出）；PK→好感度变化（胜利 +10 / 失败安慰 / 每日 3 次防刷提示）；
- [ ] 交往等级 / 称谓 / 等级解锁彩蛋可用（至少一条 level_up 演出 + 台词）；
- [ ] 事件日志可回看；设置页角色管理面板可用（召唤/送走/重命名/关系进度）；
- [ ] `cargo test -p qibai-core` 全过（既有 32 项 + 新增全部通过），迁移幂等（旧 settings.json 无 personas.json 时首次启动自动初始化）；
- [ ] README.md / CHECKLIST.md 更新（新增 P0+P1 说明与验收行）；`元气小小_MVP_源码包.zip` 重打包（排除 target）；
- [ ] 冒烟截图存 `artifacts/`（召唤/串门/猜拳/好感度/日志/设置面板 ≥ 6 张）。

## 七、交付物

- 代码变更（core：新增 personas/social 模块 + engine 门面 + persona 性格参数化 + 迁移；前端：双角色同屏与互动 + 设置页面板 + mock 扩展）；
- 测试记录（`cargo test` 输出，注明新旧用例数）；
- 冒烟截图（`artifacts/` 新增 ≥ 6 张）；
- 文档更新（README / CHECKLIST / 本文件保留为 `docs/P0P1_SOCIAL_PLAN.md`）；
- `元气小小_MVP_源码包.zip`（含源码、资产、docs、截图，排除 `qibai/target`）。

## 八、工作包划分

| 包 | 负责 | 交付 |
|---|---|---|
| 包 1 | qibai-core（P0 实体化 + P1 数据与规则） | core 代码 + 单测全过 + 迁移 |
| 包 2 | 前端（双角色同屏 + 互动 + 设置页面板 + mock） | 前端代码 + 冒烟截图 |
| 包 3 | 集成交付（回归 + README/CHECKLIST + zip + 汇总截图） | 文档 + zip + 回归记录 |

依赖：包 3 依赖包 1、2 完成；包 1、2 并行（本契约即对齐基准）。
