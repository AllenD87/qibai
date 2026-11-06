# 绘画提示词（promptgen）· com.qibai.skill.promptgen

把用户口述的主体/风格/光照/镜头，按固定模板拼成英文 AI 绘画 prompt + 通用负面词 + 中文对照。中英双语，零权限纯本地。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.promptgen` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | 无（纯本地，无网络/文件/命令权限） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「生成提示词」「画图提示词」「帮我写个绘画描述」「prompt」 |

## 文件清单

```
promptgen/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 关键词映射模板脚本（无网络依赖）
├── SKILL.md            # WorkBuddy：frontmatter + Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明
└── README.md           # 本文档
```

## 来源参考

- 素材源：linexjlin/GPTs `Diffusion Master.md`（prompt 结构顺序：主体→类型→风格→灵感→镜头→渲染细节）；yang0/handraw-style（001–366 手绘风格编号 + 中英双语 prompt 对照）；VoltAgent/awesome-claude-code-subagents `categories/06-developer-experience/visual-asset-generator.md`（必备槽位：光照+媒介+调色板+镜头+分辨率细节词）；ecnu-icalk/structured-ai-painting-expert（Positive/Negative/中文释义三段式输出）。
- 仓库：https://github.com/linexjlin/GPTs/blob/main/prompts/Diffusion%20Master.md ；https://github.com/yang0/handraw-style ；https://skillmd.com/skills/ecnu-icalk/structured-ai-painting-expert
- 本包对应方法论详见 `_backup/research-v18/creative-research.md` 的「AI 绘画提示词生成」节。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 要求 | 本包实现 |
|---|---|
| SKILL.md frontmatter（9 字段） | ✅ 齐全 |
| 正文步骤化指令 | ✅ Trigger/Instructions/Example + Changelog |
| config.yaml（name/version/description/triggers/category） | ✅ triggers 6 条，三处触发词一致 |
| assets/ 资源目录 | ✅ |
| 上架前安全扫描硬标准 | ✅ 见安全自查表 |

## 权限与安全设计

- **零权限声明**：不申请任何权限，词库与模板全部内嵌。
- 无硬编码密钥：不接任何绘图 API，prompt 文本交给用户自己的绘图工具。
- 内容合规：风格只用通用流派词（水彩/油画/赛博朋克/水墨等），**不写在世艺术家姓名**（规避肖像/版权争议）、不写真实人名；负面词内置 `nsfw, nude, naked, sexual, weapon, blood, gore`。
- 输入安全：用户描述仅做 case 模式匹配，超长自动截断展示，不进任何命令。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 无任何密钥/API |
| 2 | 无数据外泄指令 | ✅ 无网络调用、无文件读取 |
| 3 | 无越权系统命令 | ✅ 仅 sed/cut/grep 文本处理 |
| 4 | 无不安全 API 调用 | ✅ 无 API 调用 |
| 5 | 命令注入防护 | ✅ 描述仅用于 case 字符串匹配 |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 空参给用法示例；未命中关键词给通用模板 |

## 完善点（相对原素材）

- 原素材（Diffusion Master）把在世艺术家名字当风格标签 → 改为**通用流派词**，规避版权争议。
- 原素材需用户懂英文槽位语法 → 改为**中文口述即可**：脚本本地中英关键词映射，自动拼英文 prompt。
- 新增：固定负面词清单（含 nsfw/暴力词屏蔽）+ 中文对照一行读懂。
- 新增：未命中关键词时不硬编，回退通用模板并提示可补充的细节维度。

## 语音调用示例

- 「元气小小，生成提示词：一只橘猫，水彩风，黄昏光」
- 「元气小小，画图提示词怎么写，我想画雪山特写」
- 「元气小小，帮我写个绘画描述」→ 命中 promptgen（提示补充主体/风格/光照/镜头）

## 自测结果

- `bash -n run.sh` ✅；实跑：`{"subject":"一只橘猫"}` 命中猫主体、「雪山+水彩+黄昏+特写」四槽全命中、空参给用法示例、未命中描述回退通用模板，输出均 ≤2000 字符。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
