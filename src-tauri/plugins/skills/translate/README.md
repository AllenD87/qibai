# 翻译助手（translate）· com.qibai.skill.translate

中英互译小助手：脚本查本地常用词表给术语参考，主模型整句翻译并润色。纯本地处理，不联网。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.translate` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | 无（纯本地，无网络/文件权限） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「翻译一下：你好」「把这句话翻成英文」「这个用英语怎么说：我稍后回复你」 |

## 文件清单

```
translate/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 可执行脚本：方向识别 + 本地词表整句命中 + 术语参考
├── SKILL.md            # WorkBuddy：frontmatter + Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明
└── README.md           # 本文档
```

## 来源参考

- [Moksa1123/taiwan-translate-skill](https://github.com/Moksa1123/taiwan-translate-skill)，原始文件 `translate-cli/assets/taiwan-translate/SKILL.md`（方法论：模型翻译整句 + 脚本查词汇表校验，validator 只报告不修改）。
- [laolaoshiren/claude-code-skills-zh](https://github.com/laolaoshiren/claude-code-skills-zh)（中文库原创技能），原始文件 `skills/error-translator/SKILL.md`（输出格式：原文/译文分行，技术术语保留原文）。
- 本包对应条目与抽取的方法论详见 `_backup/research-v18/skills-research.md` 的 §2。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 要求 | 本包实现 |
|---|---|
| SKILL.md frontmatter（name/display_name/display_name_en/description/description_zh/description_en/version/author/examples_zh） | ✅ 9 字段齐全 |
| 正文步骤化指令 | ✅ Trigger/Instructions/Example + Changelog |
| config.yaml（name/version/description/triggers/category） | ✅ triggers 6 条，与 SKILL.md、functions 描述三处一致 |
| assets/ 资源目录 | ✅ |
| 上架前安全扫描硬标准 | ✅ 见下方安全自查表 |

## 权限与安全设计

- **零权限声明**：不申请 network:read / files:read，从根上杜绝外泄——原文不出本机。
- 反「子串自动替换」：词表只做**整句精确命中**与**术语参考列出**，绝不把词条译文拼进整句（素材 §2 记录的旧版 bug 教训）。
- 脚本「只报告不改写」：整句翻译归主模型，脚本不回写任何内容、不修改原文。
- 无任何密钥/API 调用，离线可用。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 词表即普通短语，无任何密钥 |
| 2 | 无数据外泄指令 | ✅ 无网络调用，原文不发送任何外部服务 |
| 3 | 无越权系统命令 | ✅ 仅字符串匹配，无写/删操作 |
| 4 | 无不安全 API 调用 | ✅ 无 API 调用 |
| 5 | 命令注入防护 | ✅ text 只做字符串比较与回显，不进 shell 拼接 |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 空参数/超长文本截断后均友好中文提示，exit 0 |

## 完善点（相对原素材）

- 原素材词表 2,300+ 词且依赖 .po 文件工作流 → 本包裁剪为桌面语音场景最常用的 60 条双语对照，整句可直接命中。
- 原素材自动代换整句产生乱码的教训 → 本包明确禁止子串替换，术语只列出供主模型核对。
- 原素材未强制专业文本人工复核 gate → 本包在 Instructions 与脚本输出中均提示「法律/医疗文本需人工复核」。
- 新增：自动方向识别（按是否含汉字判定中→英/英→中），语音场景免用户指定方向。

## 语音调用示例

- 「元气小小，翻译一下：你好」
- 「元气小小，把这句话翻成英文：我稍后回复你」
- 「元气小小，这个用英语怎么说：会议几点开始」

## 自测结果

- `bash -n run.sh` ✅；实跑：中→英整句命中（你好）、英→中整句命中（as soon as possible）、未收录句子给术语参考、空参数优雅提示，均 exit 0。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
