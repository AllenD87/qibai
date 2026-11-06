# 内容创作专家（Content Creator）· com.qibai.expert.contentcreator

从品牌语气出发做选题规划：产出话题簇清单、内容大纲与多套标题方案，用量化目标驱动内容质量与转化。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.expert.contentcreator` |
| 类型 | expert |
| 版本 | 1.0.0 |
| 权限声明 | 无（仅引用技能白名单） |
| 技能白名单 | `com.qibai.skill.writingpolish` |
| 语音触发 | 「帮我做一份选题清单」「帮我给这个选题写大纲和标题」「帮我排一下下个月的内容日历」 |

## 文件清单

```
content-creator/
├── qibai-plugin.json              # 元气小小清单（system_prompt + 技能白名单）
├── .codebuddy-plugin/plugin.json   # WorkBuddy 专家核心配置（官方隐藏目录，带前导点）
├── agents/contentcreator.md        # WorkBuddy：frontmatter（双语）+ 系统提示词正文
├── avatars/contentcreator.png      # 512×512 PNG（约 14KB）
└── README.md                       # 本文档
```

## 来源参考

- 素材源：https://github.com/VoltAgent/awesome-claude-code-subagents
  - 主条目：`categories/08-business-product/content-marketer.md`（内容策略 / SEO 内容 / 多渠道 campaign，对口「选题+创作」）
  - 补充条目：`categories/08-business-product/landing-page-copywriter.md`（短文案）、`categories/07-specialized-domains/seo-specialist.md`（SEO 专项）
- 详见 `_backup/research-v18/experts-research.md` 的 §2（content-creator）节。
- 抽取的方法论：先品牌后内容、选题五步法（受众→Persona→内容支柱→话题簇→编辑日历）、量化目标驱动（SEO/互动率/转化阈值）、内容质量八要素。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 硬要求 | 本包实现 |
|---|---|
| `.codebuddy-plugin/plugin.json`（官方隐藏目录） | ✅ |
| `expertType:"agent"` | ✅ |
| `name` 小写连字符 + `plugin` 同值 | ✅ `qibai-contentcreator` |
| `author{name,email}` | ✅ `{Qibai, qibai@local}` |
| displayName/profession/displayDescription 双语 | ✅ `{en,zh}` 齐全 |
| displayDescription 中文 40–50 字 | ✅ 以 python `len` 实测为准 |
| tags 固定 3 个（双语） | ✅ 内容策略 / 选题规划 / 文案创作 |
| quickPrompts 固定 3 个且首条=defaultInitPrompt | ✅ |
| avatar 512×512 PNG ≤500KB | ✅ |
| categoryId（15 选 1） | ✅ `06-ContentCreative` |
| agents/*.md frontmatter（name/description 英/displayName/profession 双语/maxTurns） | ✅；无 tools 字段（系统统一分配） |

## 权限与安全设计

- 专家本身不声明权限，仅通过技能白名单引用 `writing-polish`（本地润色检查，无网络、无写文件）。
- 系统提示词含「品牌先行」原则：未确认品牌语气与目标前不启动创作，避免输出脱离品牌的文案。
- 输出定位为策略与草稿，不引导用户直接发布未经复核的内容。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 配置与提示词中无任何密钥 |
| 2 | 无数据外泄指令 | ✅ 无任何外发/上传指令 |
| 3 | 无越权系统命令 | ✅ 无命令执行引导 |
| 4 | 无不安全 API 调用 | ✅ 仅白名单技能 |
| 5 | 命令注入防护 | ✅ 不执行 shell |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 缺品牌信息先询问 |

## 完善点（相对原素材）

- 原素材面向营销 campaign 全流程（含 WebFetch/WebSearch 取数）→ 重写为「选题清单 / 大纲 / 标题方案」三段式桌面交付，剥离外网检索依赖。
- 原素材的硬阈值 checklist（SEO>80 / 互动率>5% / 转化>2%）改写为「为每条内容设定可度量目标」的柔性指引，适配非营销用户。
- 新增：双语市场元数据（displayName/profession/displayDescription/tags/quickPrompts），原素材无 WorkBuddy 市场字段。
- 新增：多渠道适配建议（公众号 / 小红书 / 微博），贴近国内创作者场景。

## 语音调用示例

- 「元气小小，帮我做一份选题清单」→ 命中 content-creator 专家
- 「元气小小，帮我给这个选题写大纲和标题」→ 大纲 + 3-5 个标题方案
- 「元气小小，帮我排一下下个月的内容日历」→ 编辑日历 + 量化目标标注

## 自测结果

- `python3 -c "import json; json.load(open('qibai-plugin.json')); json.load(open('.codebuddy-plugin/plugin.json'))"` ✅
- displayDescription 中文字数、quickPrompts[0]==defaultInitPrompt、头像尺寸与体积实测 ✅（见交付回报）
- 扫描装配与专家激活由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
