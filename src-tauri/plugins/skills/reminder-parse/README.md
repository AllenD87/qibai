# 待办解析（reminder-parse）· com.qibai.skill.reminderparse

把「明天下午3点开会」这类自然语言解析成结构化待办（日期/时间/事项），配合内置提醒功能落地。只读、无副作用。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.reminderparse` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | `reminder:set`（解析本身无副作用，声明提醒能力以配合 set_reminder） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「明天下午3点开会提醒我」「帮我记一下周五晚上8点健身」「别忘了后天交报告」 |

## 文件清单

```
reminder-parse/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 解析脚本：自然语言 → 结构化 JSON（GNU date 归一化）
├── SKILL.md            # WorkBuddy：frontmatter + Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明
└── README.md           # 本文档
```

## 来源参考

- 素材源：alirezarezvani/claude-skills（`productivity/meetings` 的 action_item_extractor：owner+due-date 抽取、ORPHAN/NO-DUE 标记思路）——是社区中最接近的条目；社区无精确「中文自然语言时间解析」技能命中，已如实记录。
- 方法论要点：脚本只做确定性抽取、不自动写日历；歧义项反问；本包在此基础上扩展了中文时间词（今天/明天/后天/周X/下周X + 时段/钟点）与 ISO 归一化。
- 详见 `_backup/research-v18/skills-research.md` 的「reminder-parse」节。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 要求 | 本包实现 |
|---|---|
| SKILL.md frontmatter（9 必填字段） | ✅ |
| 正文 Trigger/Instructions/Example + Changelog | ✅ |
| config.yaml（name/version/description/triggers/category） | ✅ |
| assets/ 资源目录 | ✅ |
| 上架前安全扫描硬标准 | ✅ 见安全自查表 |

## 权限与安全设计

- 只读解析：不写文件、不设提醒（提醒由主模型调用内置 `set_reminder` 完成）。
- 无网络、无密钥、无系统命令副作用。
- 参数仅做字符串匹配与日期计算（GNU date），无 shell 拼接。
- 依赖 GNU date（Linux 桌面环境自带）；输出为结构化 JSON，异常输入输出 `ok:false` 优雅失败。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ |
| 2 | 无数据外泄指令 | ✅ 无网络调用 |
| 3 | 无越权系统命令 | ✅ 仅 date 计算 |
| 4 | 无不安全 API 调用 | ✅ |
| 5 | 命令注入防护 | ✅ 参数仅字符串匹配 |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 空参/无法解析输出 ok:false JSON |

## 完善点（相对原素材）

- 原素材为英文 action-item 抽取、面向会议纪要 → 重写为中文自然语言时间解析，支持今天/明天/后天/大后天、周X/下周X、HH:MM/X点/X点半、上午/下午/晚上。
- 新增：GNU date 归一化为 ISO（when_iso），主模型可直接用于 set_reminder。
- 新增：事项提取（剥离时间/日期词与填充词），输出 JSON 三处一致（SKILL.md/config.yaml/functions）。

## 语音调用示例

- 「元气小小，明天下午3点开会提醒我」→ reminder-parse → set_reminder
- 「元气小小，帮我记一下周五晚上8点健身」

## 自测结果

- `bash -n run.sh` ✅；实跑：今天/明天/后天/下周一/纯文本输入均正确输出 JSON，空参数输出 ok:false。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
