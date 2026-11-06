# 创意命名（namegen）· com.qibai.skill.namegen

按场景（科技/文艺/萌宠/游戏/国风）随机组合两字词根，输出 5 个候选名 + 逐字寓意注解。只讲寓意与音韵，不碰八字五格，零权限纯本地。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.namegen` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | 无（纯本地，无网络/文件/命令权限） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「起个名字」「取个名」「帮我命名」「给产品起名」 |

## 文件清单

```
namegen/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 词根库组合脚本（无网络依赖）
├── SKILL.md            # WorkBuddy：frontmatter + Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明
└── README.md           # 本文档
```

## 来源参考

- 素材源：phuryn/pm-skills 仓 `product-name` 技能（输出固定 5 个候选名、每个带 Rationale/Brand Fit/Memorability 注解思路）；poemBabyName（诗经/楚辞等典籍拆词组合思路）；mingzi-ts（高频常用字组合、避免生僻字）。
- 仓库：https://github.com/phuryn/pm-skills/blob/main/pm-marketing-growth/skills/product-name/SKILL.md ；https://gitee.com/badcodemaker/poemBabyName ；https://www.npmjs.com/package/mingzi-ts
- 本包对应方法论详见 `_backup/research-v18/creative-research.md` 的「创意命名器」节。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 要求 | 本包实现 |
|---|---|
| SKILL.md frontmatter（9 字段） | ✅ 齐全 |
| 正文步骤化指令 | ✅ Trigger/Instructions/Example + Changelog |
| config.yaml（name/version/description/triggers/category） | ✅ triggers 6 条，三处触发词一致 |
| assets/ 资源目录 | ✅ |
| 上架前安全扫描硬标准 | ✅ 见安全自查表 |

## 权限与安全设计

- **零权限声明**：不申请任何权限，词根库全部内嵌。
- 无硬编码密钥：不接任何命名 API。
- 内容合规：注解只讲寓意/音韵/好记，**剔除原素材的八字/三才五格/五行测算**；用词全部为常用字，不取生僻字；不做「这个名字肯定没被注册」式承诺，自带商标/域名自查提示。
- 随机组合用时间戳种子 + 互质偏移，5 个候选两两不重复。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 无任何密钥/API |
| 2 | 无数据外泄指令 | ✅ 无网络调用、无文件读取 |
| 3 | 无越权系统命令 | ✅ 仅 sed/cut 文本处理 |
| 4 | 无不安全 API 调用 | ✅ 无 API 调用 |
| 5 | 命令注入防护 | ✅ scene 仅用于 case 字符串匹配 |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 空参/未识别场景回退随机风格并说明 |

## 完善点（相对原素材）

- 原素材（poemBabyName / siwuxue MCP）带八字、三才五格测算 → **全部剔除**，只保留「典故拆词 + 寓意注解」。
- 原素材从古籍抠字易出「龘、靐」生僻字 → 改用常用字词根库，好念好打。
- 新增：五场景（科技/文艺/萌宠/游戏/国风）× 8 前缀 × 8 后缀，一键出 5 个不重复候选。
- 新增：末尾商标/域名自查提示，不替用户拍胸脯保证可注册。

## 语音调用示例

- 「元气小小，起个名字，要萌宠风」
- 「元气小小，帮我给产品命名」→ 命中 namegen（追问风格或随机）
- 「元气小小，给我家猫取个名」

## 自测结果

- `bash -n run.sh` ✅；实跑：`{"scene":"宠物"}` 命中萌宠库、`{"scene":"科技产品"}` 命中科技库、空参随机回退并说明；5 个候选两两不重复；输出 ≤2000 字符。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
