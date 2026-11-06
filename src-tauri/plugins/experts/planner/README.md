# 日程规划专家（planner）· com.qibai.expert.planner

把目标、任务与会议安排成清晰可执行的日程：优先级排序、时间块规划、提醒设置。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.expert.planner` |
| 类型 | expert |
| 版本 | 1.0.0 |
| 权限声明 | 无（仅引用技能白名单） |
| 技能白名单 | `com.qibai.skill.reminderparse`、内置 `com.qibai.set_reminder` |
| 语音触发 | 「帮我安排明天的日程」「帮我把这周的任务排个优先级」「帮我设置提醒」 |

## 文件清单

```
planner/
├── qibai-plugin.json           # 元气小小清单（system_prompt + 技能白名单）
├── .codebuddy-plugin/plugin.json  # WorkBuddy 专家核心配置（官方隐藏目录，带前导点）
├── agents/planner.md           # WorkBuddy：frontmatter（双语）+ 系统提示词正文
├── avatars/planner.png         # 512×512 PNG（10KB）
└── README.md                   # 本文档
```

## 来源参考

- 素材源：https://github.com/VoltAgent/awesome-claude-code-subagents
  - 主条目：`categories/09-meta-orchestration/task-distributor.md`（任务拆解/优先级/截止期/负载均衡方法论）
  - 补充条目：`categories/08-business-product/project-manager.md`（时间线/关键路径/里程碑/缓冲管理）
- 抽取的方法论：四步拆解法（收集→画像→选策略→成稿）、负载均衡/优先级策略显式化论证、截止期政策与兜底策略、**边界诚实原则**（不编造时间与指标，缺输入声明假设）。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 硬要求 | 本包实现 |
|---|---|
| `.codebuddy-plugin/plugin.json`（官方隐藏目录） | ✅ |
| `expertType:"agent"` | ✅ |
| `name` 小写连字符 + `plugin` 同值 | ✅ `qibai-planner` |
| `author{name,email}` | ✅ |
| displayName/profession/displayDescription 双语 | ✅ `{en,zh}` 齐全 |
| displayDescription 中文 40–50 字 | ✅ 49 字 |
| tags 固定 3 个（双语） | ✅ 日程规划/时间管理/提醒设置 |
| quickPrompts 固定 3 个且首条=defaultInitPrompt | ✅ |
| avatar 512×512 PNG ≤500KB | ✅ 10KB |
| categoryId（15 选 1） | ✅ `10-ProjectQuality` |
| agents/*.md frontmatter（name/description 英/displayName/profession 双语） | ✅；无 tools 字段（系统统一分配） |

## 权限与安全设计

- 专家本身不声明权限，仅通过技能白名单引用 `reminder-parse`（只读解析）与内置 `set_reminder`（用户授权后生效）。
- 系统提示词含「边界诚实」原则：不编造时间/指标，缺输入先询问——避免误导性日程。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ |
| 2 | 无数据外泄指令 | ✅ 无任何外发指令 |
| 3 | 无越权系统命令 | ✅ 无命令执行 |
| 4 | 无不安全 API 调用 | ✅ 仅白名单技能 |
| 5 | 命令注入防护 | ✅ 不执行 shell |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 缺输入先询问 |

## 完善点（相对原素材）

- 原 task-distributor 面向多 agent 负载均衡（队列/执行者画像）→ 重写为**个人日程**场景（四象限优先级 + 时间块 + 缓冲 + 提醒落地）。
- 新增：与待办解析技能、内置提醒功能的联动白名单；双语市场字段补齐（原素材无 WorkBuddy 市场元数据）。
- 新增：输出规范（日程表 + 提醒建议 + 待确认问题），保证结论可执行。

## 语音调用示例

- 「元气小小，帮我安排明天的日程」→ 命中 planner 专家
- 「元气小小，帮我把这周的任务排个优先级」
- 「元气小小，帮我给下午的会议设置提醒」→ planner → reminder-parse → set_reminder

## 自测结果

- `python3 -c "import json,yaml; json.load(open('.codebuddy-plugin/plugin.json'))"` ✅
- 头像 512×512、10KB ≤500KB ✅
- 扫描装配与专家激活由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
