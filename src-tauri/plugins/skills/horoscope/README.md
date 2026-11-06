# 星座运势（horoscope）· com.qibai.skill.horoscope

按星座查看今日运势：总体评分、幸运色、幸运数字、事业/爱情/健康小贴士。纯本地规则生成，娱乐向，全龄友好。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.horoscope` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | 无（纯本地，无网络/文件/命令权限） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「看看白羊座今天怎么样」「帮我看看今天的运势」「我的星座今天运气如何」 |

## 文件清单

```
horoscope/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 本地规则生成脚本（无网络依赖）
├── SKILL.md            # WorkBuddy：frontmatter + Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明
└── README.md           # 本文档
```

## 来源参考

- 素材源：VoltAgent/awesome-agent-skills（`SKILLS.md` 全量技能库）中占星/运势类条目，以及社区中文技能库的「每日运势」类技能。
- 原素材特点：多数依赖外部占星 API（需要 key、有网络依赖与数据外泄风险）或让 LLM 即兴编造（结果不稳定、易绝对化）。
- 本包对应条目与抽取的方法论详见 `_backup/research-v18/creative-research.md` 的「星座运势」节。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 要求 | 本包实现 |
|---|---|
| SKILL.md frontmatter（name/display_name/display_name_en/description/description_zh/description_en/version/author） | ✅ 9 字段齐全 |
| 正文步骤化指令 | ✅ Trigger/Instructions/Example + Changelog |
| config.yaml（name/version/description/triggers/category） | ✅ |
| assets/ 资源目录 | ✅ |
| 上架前安全扫描硬标准 | ✅ 见安全自查表 |

## 权限与安全设计

- **零权限声明**：不申请 network:read / files:read / command:run，从根上杜绝外泄与越权。
- 无硬编码密钥：不接任何 API，无 key。
- 内容合规：文案全部正向、通用、全龄友好；不绝对化预测、不引导重大决策，自带「娱乐参考」免责声明。
- 确定性随机：以「年中第几天 + 星座序号」为种子，同一天同一星座结果稳定可复现。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 无任何密钥/API |
| 2 | 无数据外泄指令 | ✅ 无网络调用、无文件读取 |
| 3 | 无越权系统命令 | ✅ 仅 date 取日期 |
| 4 | 无不安全 API 调用 | ✅ 无 API 调用 |
| 5 | 命令注入防护 | ✅ 参数仅用于字符串匹配 |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 空参数/未知星座列出可选项 |

## 完善点（相对原素材）

- 原素材依赖外部 API/密钥 → 改为**纯本地规则生成**，零权限、零外泄、离线可用。
- 原素材预测绝对化（「今天会发财」）→ 改为温和建议式文案 + 娱乐免责声明。
- 新增：确定性随机种子，结果当日稳定，便于语音重复播报一致。

## 语音调用示例

- 「元气小小，使用星座运势，看看白羊座今天怎么样」
- 「元气小小，帮我看看今天的运势」→ 命中 horoscope（追问星座）

## 自测结果

- `bash -n run.sh` ✅；实跑：12 星座均可生成、空参数/未知星座优雅提示、同一天结果稳定、无网络调用。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
