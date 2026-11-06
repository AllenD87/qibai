# 代码审查（code-review）· com.qibai.skill.codereview

只读静态扫描本地代码目录，命中危险模式清单时输出「文件:行号 + 风险类型 + 修复建议」。全程只读，不改任何文件。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.codereview` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | `files:read`（最小集，只读） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「审查这段代码：projects/my-app」「检查代码有没有安全隐患」 |

## 文件清单

```
code-review/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 可执行脚本：路径校验 + 危险模式只读扫描
├── SKILL.md            # WorkBuddy：frontmatter + Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明
└── README.md           # 本文档
```

## 来源参考

- [obra/superpowers](https://github.com/obra/superpowers)，原始文件 `skills/requesting-code-review/SKILL.md`（方法论：审查时机分级、问题三级分流 Critical/Important/Minor）。
- [laolaoshiren/claude-code-skills-zh](https://github.com/laolaoshiren/claude-code-skills-zh)（中文库原创技能），原始文件 `skills/zh-code-reviewer/SKILL.md`（方法论：报告每条带 `[文件:行号]` + 💡修复建议，五维清单含安全）。
- 本包对应条目与抽取的方法论详见 `_backup/research-v18/skills-research.md` 的 §4。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 要求 | 本包实现 |
|---|---|
| SKILL.md frontmatter 9 字段 | ✅ 齐全（含 examples_zh 2 条） |
| 正文步骤化指令 | ✅ Trigger/Instructions/Example + Changelog |
| config.yaml（name/version/description/triggers/category） | ✅ triggers 6 条，三处一致 |
| assets/ 资源目录 | ✅ |
| 上架前安全扫描硬标准 | ✅ 见下方安全自查表 |

## 权限与安全设计

- **最小权限**：仅 `files:read`，无 network:read、无写权限；脚本只读 grep，无任何写/删/改。
- **路径校验**：拒绝含 `..` 的路径；相对路径按 `$HOME` 解析，绝对路径必须落在 `$HOME` 内；目录不存在给中文提示。
- **扫描降噪**：默认排除 node_modules / .git / venv / dist / build 等依赖与产物目录，只扫常见代码后缀。
- **只报告不修复**：脚本输出风险清单，修复动作由用户/主模型在对话中给出建议文本，脚本本身不动代码。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 脚本无密钥；反而专扫用户代码里的硬编码密钥 |
| 2 | 无数据外泄指令 | ✅ 无网络调用，结果只打 stdout |
| 3 | 无越权系统命令 | ✅ 仅只读 grep，无写/删操作 |
| 4 | 无不安全 API 调用 | ✅ 无 API 调用 |
| 5 | 命令注入防护 | ✅ path 经白名单校验后仅作 grep 位置参数（双引号包裹） |
| 6 | 路径遍历防护 | ✅ 拒 `..`，相对/绝对路径均限 `$HOME` 内 |
| 7 | 异常输入优雅失败 | ✅ 空参/越界/目录不存在均友好中文提示，exit 0 |

## 完善点（相对原素材）

- 原素材让模型直接读全仓库、无文件范围限制 → 本包改为「用户指定目录 + 路径校验 + 依赖目录排除」的确定性扫描。
- 原素材只泛泛提 SQL 注入/XSS → 本包落地为 6 类可执行危险模式（eval/命令拼接/rm -rf/硬编码密钥/无 WHERE SQL/curl|sh）。
- 原素材报告无分级门槛 → 本包按 Critical/Important/Minor 三级分流，并要求每条 findings 必带 文件:行号 + 修复建议。
- 新增：扫描即声明只读，不替用户改代码，避免审查工具本身变成破坏面。

## 语音调用示例

- 「元气小小，审查这段代码：projects/my-app」
- 「元气小小，帮我看看这个项目有没有安全隐患」
- 「元气小小，代码检查一下 scripts 目录」

## 自测结果

- `bash -n run.sh` ✅；实跑：含危险模式的样例目录正确输出 文件:行号+建议，干净目录输出「未发现高危模式」，`..` 路径与越界绝对路径被拒绝，不存在目录友好提示，均 exit 0。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
