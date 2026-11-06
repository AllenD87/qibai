# 数据分析专家（Data Analyst）· com.qibai.expert.dataanalyst

先定义指标口径再分析：选用留存、漏斗、分群等分析框架，把数字讲成清晰的业务洞察与行动建议。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.expert.dataanalyst` |
| 类型 | expert |
| 版本 | 1.0.0 |
| 权限声明 | 无（仅引用技能白名单） |
| 技能白名单 | `com.qibai.skill.filesearch` |
| 语音触发 | 「帮我分析一下这份数据」「帮我定一下看板的指标口径」「帮我看看转化掉在哪一步」 |

## 文件清单

```
data-analyst/
├── qibai-plugin.json              # 元气小小清单（system_prompt + 技能白名单）
├── .codebuddy-plugin/plugin.json  # WorkBuddy 专家核心配置（官方隐藏目录，带前导点）
├── agents/dataanalyst.md          # WorkBuddy：frontmatter（双语）+ 系统提示词正文
├── avatars/dataanalyst.png         # 512×512 PNG（约 16KB）
└── README.md                       # 本文档
```

## 来源参考

- 素材源：https://github.com/VoltAgent/awesome-claude-code-subagents
  - 主条目：`categories/05-data-ai/data-analyst.md`（与插件同名：BI / 统计分析 / 可视化 / 决策支持，完全对口）
  - 补充条目：`categories/10-research-analysis/cohort-analysis.md`、`categories/10-research-analysis/ab-test-analysis.md`（归因 / 分层场景）、`categories/05-data-ai/sql-pro.md`（查询层）
- 详见 `_backup/research-v18/experts-research.md` 的 §3（data-analyst）节。
- 抽取的方法论：先业务后数据、指标定义先行（口径/数据源/刷新频率/owner/成功标准）、分析工具箱（Cohort/Funnel/留存/分群/A-B/归因）、数据叙事框架、质量红线（查询时限/统计显著性）。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 硬要求 | 本包实现 |
|---|---|
| `.codebuddy-plugin/plugin.json`（官方隐藏目录） | ✅ |
| `expertType:"agent"` | ✅ |
| `name` 小写连字符 + `plugin` 同值 | ✅ `qibai-dataanalyst` |
| `author{name,email}` | ✅ `{Qibai, qibai@local}` |
| displayName/profession/displayDescription 双语 | ✅ `{en,zh}` 齐全 |
| displayDescription 中文 40–50 字 | ✅ 以 python `len` 实测为准 |
| tags 固定 3 个（双语） | ✅ 指标体系 / 分析框架 / 数据叙事 |
| quickPrompts 固定 3 个且首条=defaultInitPrompt | ✅ |
| avatar 512×512 PNG ≤500KB | ✅ |
| categoryId（15 选 1） | ✅ `04-DataAI` |
| agents/*.md frontmatter（name/description 英/displayName/profession 双语/maxTurns） | ✅；无 tools 字段（系统统一分配） |

## 权限与安全设计

- 专家本身不声明权限，仅通过技能白名单引用 `file-search`（限定用户目录、排除敏感路径的只读搜索）。
- 系统提示词含「指标定义先行」与「质量红线」：口径不一致不下结论、统计不显著如实说明，不臆造数字。
- 不输出任何数据库连接串 / 凭据 / 批量导出指令，避免数据外泄。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 配置与提示词中无任何密钥 |
| 2 | 无数据外泄指令 | ✅ 无外发/批量导出指令 |
| 3 | 无越权系统命令 | ✅ 无命令执行引导 |
| 4 | 无不安全 API 调用 | ✅ 仅白名单技能 |
| 5 | 命令注入防护 | ✅ 不执行 shell |
| 6 | 路径遍历防护 | ✅ 文件搜索由白名单技能限定根目录 |
| 7 | 异常输入优雅失败 | ✅ 口径缺失先询问/声明假设 |

## 完善点（相对原素材）

- 原素材面向 BI 看板开发（含 SQL/看板工具链）→ 重写为「指标口径说明 + 分析框架 + 可视化建议」的咨询式交付，剥离具体工具链依赖。
- 保留并强调质量红线：查询轻量快速、统计显著性必验、边缘情况必测、不显著如实说明。
- 新增：双语市场元数据（displayName/profession/displayDescription/tags/quickPrompts），原素材无 WorkBuddy 市场字段。
- 新增：数据叙事固定结构（执行摘要 + 关键证据 + 行动建议），适配桌面伴侣轻对话场景。

## 语音调用示例

- 「元气小小，帮我分析一下这份数据」→ 命中 data-analyst 专家
- 「元气小小，帮我定一下看板的指标口径」→ 口径表格（指标/口径/数据源/owner）
- 「元气小小，帮我看看转化掉在哪一步」→ Funnel 漏斗分析建议

## 自测结果

- `python3 -c "import json; json.load(open('qibai-plugin.json')); json.load(open('.codebuddy-plugin/plugin.json'))"` ✅
- displayDescription 中文字数、quickPrompts[0]==defaultInitPrompt、头像尺寸与体积实测 ✅（见交付回报）
- 扫描装配与专家激活由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
