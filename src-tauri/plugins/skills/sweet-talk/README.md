# 情话语录（sweet-talk）· com.qibai.skill.sweettalk

三层词库本地规则生成 1-3 句温馨情话：L1 温和日常 / L2 俏皮轻松 / L3 深情克制。全龄、尊重、无擦边，零权限纯本地。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.skill.sweettalk` |
| 类型 | skill |
| 版本 | 1.0.0 |
| 权限声明 | 无（纯本地，无网络/文件/命令权限） |
| 脚本入口 | `run.sh`（`entry.command`，引擎以 `sh` 在插件目录执行） |
| 语音触发 | 「说句情话」「来一句俏皮的」「给我一句深情一点的」 |

## 文件清单

```
sweet-talk/
├── qibai-plugin.json   # 元气小小清单（引擎装配入口）
├── run.sh              # 三层词库本地规则生成脚本（无网络依赖）
├── SKILL.md            # WorkBuddy：frontmatter + Trigger/Instructions/Example + Changelog
├── config.yaml         # WorkBuddy：name/version/description/triggers/category
├── assets/README.md    # 资源目录说明
└── README.md           # 本文档
```

## 来源参考

- 素材源：Tanu-N-Prabhu/Python 仓 `pick_up_line_generator_sentiments.ipynb`（pickup-line 分类思路）与 Synergy-Networks/OpenSource 的 `rizzler.lua`——**二者均为反例素材，只取其「分层词库结构」教训，未借鉴任何一句文案**；正向措辞框架参考 Cheerme 陪伴技能「Sincere Companionship / Sincere Care without over-interference」。
- 仓库：https://github.com/Tanu-N-Prabhu/Python ；https://github.com/Synergy-Networks/OpenSource/blob/main/rizzler.lua ；https://www.toolify.ai/vi/openclaw-skills/cheerme-65418
- 本包对应方法论详见 `_backup/research-v18/creative-research.md` 的「情话语录」节。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 要求 | 本包实现 |
|---|---|
| SKILL.md frontmatter（name/display_name/display_name_en/description/description_zh/description_en/version/author/examples_zh） | ✅ 9 字段齐全 |
| 正文步骤化指令 | ✅ Trigger/Instructions/Example + Changelog |
| config.yaml（name/version/description/triggers/category） | ✅ triggers 6 条，与 SKILL.md/plugin.json 三处触发词一致 |
| assets/ 资源目录 | ✅ |
| 上架前安全扫描硬标准 | ✅ 见安全自查表与内容合规节 |

## 权限与安全设计

- **零权限声明**：不申请 network:read / files:read / command:run，从根上杜绝外泄与越权。
- 无硬编码密钥：不接任何 API，无 key。
- 词库全部人工撰写、全龄审核：只走「日常关心 + 轻量陪伴」路线。
- 随机取词用时间戳种子做确定性偏移，不依赖 `$RANDOM`，纯 POSIX 可跑。

## 内容合规

### 三层词库分级

| 层级 | 定位 | 适用场景 | 条数 |
|---|---|---|---|
| L1 温和日常 | 关心起居、提醒吃饭休息 | 任何人都能发，同事/朋友也可 | 7 |
| L2 俏皮轻松 | 小夸奖、小幽默、带点亮色 | 关系不错的朋友/同事/伴侣 | 7 |
| L3 深情克制 | 珍惜陪伴、感谢日常 | 仅亲近的人，用户明确要求时给 | 7 |

### 硬禁清单（写库与播报双重卡点）

- **身体描写**：不写身材、外貌五官特写、任何身体部位；
- **性暗示/擦边**：不出现暧昧邀约、暗示性双关、「约吗」「扑倒」类字样；
- **低俗词/土味梗**：不使用任何网络低俗话术、油腻土味谐音；
- **物化**：不把对方当作「收藏品/附属品」描述；
- **极端承诺**：严禁「永远」「一辈子」「非你不嫁/不娶」类绝对化承诺；
- **情感绑架**：不写「你不回我就……」「没有你不行」类要挟话术。

### 为何反例素材未被采用

rizzler.lua（"fell from heaven" / "rearrange the alphabet"）与 tanu notebook 的 cheesy 分类均为经典土味/油腻话术，且隐含搭讪导向；元气小小是桌面伴侣而非社交搭讪工具，用户可能是未成年人或处于工作场景，一旦出擦边句即违规。本包只借其「分层词库」结构思路，文案全部按 Cheerme「真诚陪伴、不过度干预」框架人工重写。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 无任何密钥/API |
| 2 | 无数据外泄指令 | ✅ 无网络调用、无文件读取 |
| 3 | 无越权系统命令 | ✅ 仅 date 取时间戳 |
| 4 | 无不安全 API 调用 | ✅ 无 API 调用 |
| 5 | 命令注入防护 | ✅ 参数仅用于 case 字符串匹配 |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 空参随机给一层；非法层级回退随机；条数夹回 1-3 |

## 完善点（相对原素材）

- 原素材（rizzler.lua / tanu notebook）油腻擦边 → 全部重写为全龄温馨文案，并加三层分级与硬禁清单。
- 原素材让 LLM 即兴生成（易跑偏）→ 改为**内嵌人工审核词库**，脚本只做取行，输出可控。
- 新增：层级参数 `level`（l1/l2/l3 大小写兼容）与条数 `count`（1-3），未知层级自动回退随机。
- 新增：输出末尾自带合规说明与「对方情绪低落时先问候」的使用提示。

## 语音调用示例

- 「元气小小，说句情话」
- 「元气小小，来一句俏皮的」
- 「元气小小，给我两句深情一点的」

## 自测结果

- `bash -n run.sh` ✅；实跑：`{"level":"l2"}` 命中 L2 词库、空参随机回退、非法层级回退随机、`count=3` 输出 3 句不重复；7×3 条词库逐条人工复核无擦边（详见交付回报）。
- 扫描装配与 invoke 冒烟由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
