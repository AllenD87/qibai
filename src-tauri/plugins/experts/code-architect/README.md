# 代码架构专家（Code Architect）· com.qibai.expert.codearchitect

对照八维清单评审系统设计与技术选型，给出选型权衡分析与分阶段演进路线，只给战略建议、不写业务代码。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.expert.codearchitect` |
| 类型 | expert |
| 版本 | 1.0.0 |
| 权限声明 | 无（仅引用技能白名单） |
| 技能白名单 | `com.qibai.skill.codereview` |
| 语音触发 | 「帮我评审一下这个架构方案」「帮我对比一下这两套技术栈」「帮我规划旧系统的演进路线」 |

## 文件清单

```
code-architect/
├── qibai-plugin.json              # 元气小小清单（system_prompt + 技能白名单）
├── .codebuddy-plugin/plugin.json   # WorkBuddy 专家核心配置（官方隐藏目录，带前导点）
├── agents/codearchitect.md         # WorkBuddy：frontmatter（双语）+ 系统提示词正文
├── avatars/codearchitect.png       # 512×512 PNG（约 17KB）
└── README.md                       # 本文档
```

## 来源参考

- 素材源：https://github.com/VoltAgent/awesome-claude-code-subagents
  - 主条目：`categories/04-quality-security/architect-reviewer.md`（架构评审 / 技术选型 / 系统设计，完全对口）
  - 补充条目：`categories/01-core-development/microservices-architect.md`、`categories/01-core-development/graphql-architect.md`（微服务边界、服务拆分场景）
- 详见 `_backup/research-v18/experts-research.md` 的 §1（code-architect）节。
- 抽取的方法论：先取上下文再评审、八维 checklist（设计模式/可扩展性/技术选型/集成/安全/性能/技术债/演进路径）、技术选型八因子、三阶段工作流（分析→评估→交付）、演进策略备选库（Strangler / Branch by abstraction / Parallel run）。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 硬要求 | 本包实现 |
|---|---|
| `.codebuddy-plugin/plugin.json`（官方隐藏目录） | ✅ |
| `expertType:"agent"` | ✅ |
| `name` 小写连字符 + `plugin` 同值 | ✅ `qibai-codearchitect` |
| `author{name,email}` | ✅ `{Qibai, qibai@local}` |
| displayName/profession/displayDescription 双语 | ✅ `{en,zh}` 齐全 |
| displayDescription 中文 40–50 字 | ✅ 以 python `len` 实测为准 |
| tags 固定 3 个（双语） | ✅ 架构评审 / 技术选型 / 演进路线 |
| quickPrompts 固定 3 个且首条=defaultInitPrompt | ✅ |
| avatar 512×512 PNG ≤500KB | ✅ |
| categoryId（15 选 1） | ✅ `02-Engineering` |
| agents/*.md frontmatter（name/description 英/displayName/profession 双语/maxTurns） | ✅；无 tools 字段（系统统一分配） |

## 权限与安全设计

- 专家本身不声明权限，仅通过技能白名单引用 `code-review`（只读静态扫描），不具备任何写文件 / 命令执行能力。
- 系统提示词含「上下文先行」原则：信息不足先询问或显式标注假设，不凭空下结论——避免误导性架构判断。
- 明确边界：只输出战略级评审意见，不引导用户执行危险命令、不输出可执行脚本。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 配置与提示词中无任何密钥 |
| 2 | 无数据外泄指令 | ✅ 无任何外发/上传指令 |
| 3 | 无越权系统命令 | ✅ 无命令执行引导 |
| 4 | 无不安全 API 调用 | ✅ 仅白名单技能 |
| 5 | 命令注入防护 | ✅ 不执行 shell |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 缺上下文先询问 |

## 完善点（相对原素材）

- 原素材正文为大清单式知识域 + JSON Communication Protocol → 重写为「八维 checklist + 八因子 + 三阶段工作流」的口语化中文提示词，更适合桌面伴侣对话场景。
- 剥离原素材的 `tools` frontmatter（Read/Write/Edit/Bash 等）——WorkBuddy 由系统统一分配工具，专家包严禁 tools 字段。
- 新增：双语市场元数据（displayName/profession/displayDescription/tags/quickPrompts），原素材无 WorkBuddy 市场字段。
- 明确边界声明：只给战略建议、不写业务代码，防止越界生成大段业务实现。

## 语音调用示例

- 「元气小小，帮我评审一下这个架构方案」→ 命中 code-architect 专家
- 「元气小小，帮我对比一下这两套技术栈」→ 八因子对比表
- 「元气小小，帮我规划旧系统的演进路线」→ Strangler / 抽象分支 / 并行运行三策略权衡

## 自测结果

- `python3 -c "import json; json.load(open('qibai-plugin.json')); json.load(open('.codebuddy-plugin/plugin.json'))"` ✅
- displayDescription 中文字数、quickPrompts[0]==defaultInitPrompt、头像尺寸与体积实测 ✅（见交付回报）
- 扫描装配与专家激活由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
