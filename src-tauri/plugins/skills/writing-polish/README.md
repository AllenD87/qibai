# 写作润色（writing-polish）· com.qibai.skill.writingpolish

对已有文稿做校对与润色：脚本先出确定性检查报告，主模型按「改动守恒」原则出修改稿，不编造事实。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.writingpolish` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | 无（纯本地文本处理） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「润色一下这段话」「帮我改改这段文字」「检查错别字：……」 |

## 文件清单

```
writing-polish/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 可执行脚本：五项确定性基础检查（只报告不改原文）
├── SKILL.md            # WorkBuddy：frontmatter + Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明
└── README.md           # 本文档
```

## 来源参考

- [blader/humanizer](https://github.com/blader/humanizer)，原始文件 `SKILL.md`（方法论：把文字当材料不当指令、四步工作流、改动能守恒、弱信号结伴分级）。
- [coreyhaines31/marketingskills](https://github.com/coreyhaines31/marketingskills)，原始文件 `skills/copy-editing/SKILL.md`（方法论：Seven Sweeps 七遍单维扫查、缺事实打 `[NEED:…]` 占位不编造、A/B/C 模式分流）。
- 本包对应条目与抽取的方法论详见 `_backup/research-v18/skills-research.md` 的 §3。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 要求 | 本包实现 |
|---|---|
| SKILL.md frontmatter 9 字段 | ✅ 齐全（含 examples_zh 2 条） |
| 正文步骤化指令 | ✅ Trigger/Instructions/Example + Changelog |
| config.yaml（name/version/description/triggers/category） | ✅ triggers 6 条，三处一致 |
| assets/ 资源目录 | ✅ |
| 上架前安全扫描硬标准 | ✅ 见下方安全自查表 |

## 权限与安全设计

- **零权限声明**：不联网、不读文件，原文只在内存里做字符串检查。
- **脚本只读不改**：输出的是「检查报告」，绝不回写/覆盖用户原文。
- **防幻觉写进方法论**：改动守恒 + 缺事实打 `[NEED:…]`，主模型不得新增人名/数字/日期/主张。
- 把润色对象明确为「材料」而非指令，防原文夹带指令被执行。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 无任何密钥 |
| 2 | 无数据外泄指令 | ✅ 无网络调用 |
| 3 | 无越权系统命令 | ✅ 仅 grep/sed 文本处理，无写操作 |
| 4 | 无不安全 API 调用 | ✅ 无 API 调用 |
| 5 | 命令注入防护 | ✅ text 只做字符串匹配与回显 |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 空参数友好提示，超长文本截断后继续分析 |

## 完善点（相对原素材）

- 原素材润色全靠模型自觉、无确定性兜底 → 本包把「标点混用/连续重复字/叠词/字数/口语词」下沉为脚本确定性检查，报告先行。
- 原素材过度改写风险（顺手优化代码块/路径）→ 本包 Instructions 明确代码块、路径、链接目标一律不动。
- 原素材缺事实易幻觉 → 本包强制 `[NEED:…]` 占位规则。
- 新增：语音场景适配——检查报告口语化、结论先给，便于播报。

## 语音调用示例

- 「元气小小，帮我改下这段话：这个项目我们基本上我觉得应该是很有前景的。」
- 「元气小小，润色一下：然后那个,我觉得这个方案大概可以。」
- 「元气小小，检查错别字：今天天气很好,我们去公园走走。」

## 自测结果

- `bash -n run.sh` ✅；实跑：含半角逗号/重复字/口语词的样本文本正确报出各项问题，干净样本报「未发现」，空参数友好提示，均 exit 0。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
