# 创意点子专家（idea-creator）· com.qibai.expert.ideas

先听懂你为什么想做，再发散、收敛，每次给出 2-3 个说得出理由的候选方案。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.expert.ideas` |
| 类型 | expert |
| 版本 | 1.0.0 |
| 权限声明 | 无（permissions: []） |
| 技能白名单 | `com.qibai.skill.namegen` |
| 语音触发 | 「帮我想几个创意点子」「头脑风暴一下」「给日常来点新玩法」 |

## 文件清单

```
idea-creator/
├── qibai-plugin.json              # 元气小小清单（system_prompt + namegen 白名单）
├── .codebuddy-plugin/plugin.json   # WorkBuddy 专家核心配置（官方隐藏目录，带前导点）
├── agents/ideas.md                 # WorkBuddy：frontmatter（双语）+ 系统提示词正文
├── avatars/ideas.png                # 512×512 PNG（渐变底 + 「灵感」主题字）
└── README.md                       # 本文档
```

## 来源参考

- 素材源：obra/superpowers 仓 `brainstorming` 技能 —— https://github.com/obra/superpowers/blob/main/skills/brainstorming/SKILL.md
- 辅助条目：
  - smixs/creative-director-skill（SIT/TRIZ/SCAMPER/Synectics 20+ 方法论）—— https://github.com/smixs/creative-director-skill
  - VoltAgent/awesome-claude-code-subagents `categories/10-research-analysis/first-principles-thinking.md`、`project-idea-validator.md`
- 详见 `_backup/research-v18/creative-research.md` 的「八、创意点子专家（Agent）」一节。
- 抽取的方法论（重写而非搬运）：苏格拉底式一问一答（每条消息末尾一个开放式问题，不给菜单）；先理解 why 再动手；SCAMPER 七角度发散；收敛只留 2-3 个真方案并说清 trade-off；观点归属诚实原则。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 硬要求 | 本包实现 |
|---|---|
| `.codebuddy-plugin/plugin.json`（官方隐藏目录） | ✅ |
| `expertType:"agent"` | ✅ |
| `name` 小写连字符 + `plugin` 同值 | ✅ `qibai-ideas` |
| `author{name,email}` | ✅ Qibai / qibai@local |
| displayName/profession/displayDescription 双语 | ✅ `{en,zh}` 齐全 |
| displayDescription 中文 40–50 字 | ✅ 已用 python len 校验（见交付回报） |
| tags 固定 3 个（双语） | ✅ 头脑风暴/创意发散/方案筛选 |
| quickPrompts 固定 3 个且首条=defaultInitPrompt | ✅ |
| avatar 512×512 PNG ≤500KB | ✅ |
| categoryId（15 选 1） | ✅ `06-ContentCreative` |
| agents/*.md frontmatter（name/description 英/displayName/profession 双语/maxTurns:50） | ✅；无 tools 字段（系统统一分配） |

## 权限与安全设计

- 专家本身 permissions 为空，仅通过白名单引用 `com.qibai.skill.namegen`（本地词库规则命名、零权限技能）。
- system_prompt 写明「不给选项菜单」「观点归属诚实」：避免一上来甩 5 个点子带偏用户，也避免把 AI 的猜想当用户的意思。
- 边界声明：不替用户做辞职/结婚类人生重大决策。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 纯提示词包 |
| 2 | 无数据外泄指令 | ✅ 无外发指令 |
| 3 | 无越权系统命令 | ✅ 不执行 shell |
| 4 | 无不安全 API 调用 | ✅ 仅白名单 namegen（零权限技能） |
| 5 | 命令注入防护 | ✅ 不执行 shell |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 需求模糊时先用一个问题澄清，不瞎编方案 |

## 完善点（相对原素材）

- 原 obra/superpowers 面向软件开发场景（设计文档批准后才写代码）→ 重写为通用桌面伴侣创意场景（产品/活动/日常玩法）。
- 原 smixs 列 20+ 方法论 → 只保留 SCAMPER 一把工具，避免堆术语。
- 新增：三维收敛打分（可行性/成本/差异化）显式化；与创意命名技能的联动白名单。
- 新增：WorkBuddy 双语市场字段、头像、分类、tags/quickPrompts。

## 语音调用示例

- 「元气小小，帮我想几个创意点子」→ 命中 ideas 专家
- 「元气小小，帮我头脑风暴一下新产品的方向」
- 「元气小小，给我的日常来点不一样的玩法」→ 选中命名方向时联动 namegen

## 自测结果

- `python3 -c "import json; json.load(open('qibai-plugin.json')); json.load(open('.codebuddy-plugin/plugin.json'))"` ✅
- displayDescription.zh 字数校验 ✅（见交付回报）
- quickPrompts[0] == defaultInitPrompt ✅
- 头像 512×512、≤500KB ✅
- 扫描装配与专家激活由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
