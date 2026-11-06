# 塔罗占卜（tarot）· com.qibai.skill.tarot

抽三张 22 张大阿卡纳牌看过去/现在/未来：本地确定性随机抽牌 + 温和正向解读，输出末尾固定娱乐免责声明。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.tarot` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | 无（纯本地，零权限） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「占卜」「塔罗」「抽张牌」「算一卦」 |

## 文件清单

```
tarot/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 本地抽牌与解读脚本（无网络依赖）
├── SKILL.md            # WorkBuddy：frontmatter + Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明
└── README.md           # 本文档
```

## 来源参考

- 素材源 1：Trail of Bits/skills `plugins/let-fate-decide/skills/let-fate-decide/SKILL.md` —— 「绝不假随机、牌面不凌驾安全规则、一个决策点只抽一次、逆位≠停止信号」等反迷信原则。
- 素材源 2：tarot-reader（三牌过去/现在/未来阵）、tarotoo-tarot（22 张大阿卡纳结构化词库）—— 牌阵结构与牌面释义框架。
- 本包对应条目与抽取的方法论详见 `_backup/research-v18/creative-research.md` 的「塔罗占卜」节。

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
- **确定性随机**：以「`date +%s` + 问题文本 cksum」为种子抽 3 张（脚本内去重），绝不允许模型凭感觉口述牌面（let-fate-decide 的「Never fake entropy」原则）。
- **词库内置、不接 API**：22 张大阿卡纳正/逆位解读全部本地内置，离线可用。

## 合规红线执行（§7 · horoscope/tarot 类）

- ✅ **娱乐免责声明**：固定输出在脚本末尾，逐字保留，播报不可省略。
- ✅ **不绝对化预测**：词库全部为建议式/观察式措辞，无「一定会」「千万要」「大凶」；死神牌写为「旧阶段自然收尾」，高塔牌写为「清理之后更清爽」。
- ✅ **不涉医疗/财务/法律**：免责声明明确引导咨询专业人士；SKILL.md Instructions 要求识别到重大事项时转专业渠道。
- ✅ **反迷信**：逆位 = 延迟/内化/慢一点，不做凶兆解读；一个问题只抽一次，不陪用户「抽到满意为止」。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 无任何密钥/API |
| 2 | 无数据外泄指令 | ✅ 无网络调用、无文件读取 |
| 3 | 无越权系统命令 | ✅ 仅 date/cksum 取种子 |
| 4 | 无不安全 API 调用 | ✅ 无 API 调用 |
| 5 | 命令注入防护 | ✅ question 仅展示与种子计算，不拼命令 |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 空参数默认整体抽牌，不报错 |

## 完善点（相对原素材）

- 原素材（let-fate-decide）依赖 secrets/crypto 随机源与 12 宫大阵 → 本包裁剪为 22 张大阿卡纳 + 三牌阵，适配 POSIX sh 确定性种子。
- 原素材解读常绝对化/恐吓化 → 本包词库温和正向，逆位重写为「慢一点/内化」。
- 新增：输出末尾固定免责声明 + SKILL.md 播报纪律（禁追加预测话术）。

## 语音调用示例

- 「元气小小，用塔罗帮我抽三张牌，问问换工作的事」
- 「元气小小，抽张牌，算一卦」
- 「元气小小，今天的运势，塔罗看看」→ 命中 tarot（question 为空，整体状态抽牌）

## 自测结果

- `bash -n run.sh` ✅；实跑：带问题/空参均可抽牌、三牌不重复、末尾免责声明必现、多次运行结果不同但同参同刻可复现、无网络调用。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
