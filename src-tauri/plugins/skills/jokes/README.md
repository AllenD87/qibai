# 冷笑话（jokes）· com.qibai.skill.jokes

本地词库生成冷笑话：谐音/文字游戏/反转三类，按类别或随机抽 1-3 条，全龄无伤害。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.jokes` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | 无（纯本地，零权限） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「讲个笑话」「冷笑话」「来段子」「逗我笑」 |

## 文件清单

```
jokes/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 本地词库选段脚本（无网络依赖）
├── SKILL.md            # WorkBuddy：frontmatter + Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明
└── README.md           # 本文档
```

## 来源参考

- 素材源 1：Sv443/JokeAPI（开源 REST 笑话 API，MIT）—— 7 类分类槽位 + 6 个 blacklist flag（nsfw/religious/political/racist/sexist/explicit）+ safe-mode 开关的安全机制设计。
- 素材源 2：icanhazdadjoke（`{id, joke}` 静态库 + 随机抽一条的数据形态）。
- 本包对应条目与抽取的方法论详见 `_backup/research-v18/creative-research.md` 的「冷笑话」节。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 要求 | 本包实现 |
|---|---|
| SKILL.md frontmatter（name/display_name/display_name_en/description/description_zh/description_en/version/author） | ✅ 9 字段齐全（含 examples_zh） |
| 正文步骤化指令 | ✅ Trigger/Instructions/Example + Changelog |
| config.yaml（name/version/description/triggers/category） | ✅ triggers 6 条，三处一致 |
| assets/ 资源目录 | ✅ |
| 上架前安全扫描硬标准 | ✅ 见安全自查表 |

## 权限与安全设计

- **零权限声明**：不申请 network:read / files:read，从根上杜绝外泄与越权。
- **静态词库**：不接 JokeAPI 的 REST 调用（避免 HTTP 注入链与网络依赖），段子全部本地内置。
- **safe-mode 词库**：入库前逐条筛除 6 类国际黑名单 + 3 类本土高发雷区（地域黑/职业黑/疾病玩笑），只留文字游戏/反转/谐音。

## 合规红线执行（§7 · jokes 类）

- ✅ **禁地域/职业/疾病/性别/群体攻击**：词库 27 条全部自查通过，无任何群体指向；「程序员」仅出现在无害的 in-ZEN 谐音梗中，且无贬低。
- ✅ **禁低俗梗**：谐音类均为日常双关，无身体部位/性暗示；「零蛋」「秘书」类已替换掉敏感双关。
- ✅ **禁时政梗**：不涉人物、宗教、政治。
- ✅ **播报纪律**：SKILL.md Instructions 要求不自行即兴扩展段子，用户要「损人/黄」时婉拒并引导安全类别。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 无任何密钥/API |
| 2 | 无数据外泄指令 | ✅ 无网络调用、无文件读取 |
| 3 | 无越权系统命令 | ✅ 仅 date/cksum/sed/grep |
| 4 | 无不安全 API 调用 | ✅ 无 API 调用 |
| 5 | 命令注入防护 | ✅ category 仅字符串匹配，不拼命令 |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 空参默认随机输出；未知类别友好列出可选项 |

## 完善点（相对原素材）

- 原素材是 REST API（需联网、有网络调用链注入面）→ 本包改为**本地静态词库 + 确定性种子抽段**，离线可用。
- 原素材分类含 Dark/Pun 等高风险类目 → 本包只保留三类无伤害槽位，并把 6+3 黑名单作为词库准入标准。
- 新增：同批抽段去重、类别命中即限定该类、空参默认混合输出（适配语音无参场景）。

## 语音调用示例

- 「元气小小，讲个冷笑话逗我笑」
- 「元气小小，来个谐音梗段子」→ 命中 jokes（category=谐音）
- 「元气小小，讲个反转类的」→ 命中 jokes（category=反转）

## 自测结果

- `bash -n run.sh` ✅；实跑：`{}` 空对象默认输出、三类类别分别命中、未知类别友好提示、同批段子不重复、无网络调用。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
