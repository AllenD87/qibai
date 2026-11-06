# 极客玩法专家（geek-play）· com.qibai.expert.geek

只在用户明确想玩时，端出 figlet、cowsay、fortune 这类只读终端彩蛋与 ASCII 趣味玩法，绝不碰真实系统。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.expert.geek` |
| 类型 | expert |
| 版本 | 1.0.0 |
| 权限声明 | 无（permissions: []） |
| 技能白名单 | 无（[]） |
| 语音触发 | 「玩点好玩的」「来个终端彩蛋」「极客玩法」 |

## 文件清单

```
geek-play/
├── qibai-plugin.json              # 元气小小清单（system_prompt 含只读红线）
├── .codebuddy-plugin/plugin.json   # WorkBuddy 专家核心配置（官方隐藏目录，带前导点）
├── agents/geek.md                  # WorkBuddy：frontmatter（双语）+ 系统提示词正文
├── avatars/geek.png                # 512×512 PNG（渐变底 + 「极客」主题字）
└── README.md                       # 本文档
```

## 来源参考

- 素材源：agarrharr/awesome-cli-apps 的「Just for Fun」分类 —— https://github.com/agarrharr/awesome-cli-apps
- 辅助条目：
  - `cowsay` / `ponysay` / `yosay`（ASCII 对话框角色系列）
  - `figlet`（大字 ASCII 艺术字）、`fortune` / `fortune-mod`（随机语录饼干）
  - anthropics/algorithmic-art（可复现随机思路）
- 详见 `_backup/research-v18/creative-research.md` 的「十、极客玩法专家（Agent）」一节。
- 抽取的方法论（重写而非搬运）：Just for Fun 清单（figlet/cowsay/fortune/ASCII art/别名建议）；只读安全边界（不写不删不联网、不执行危险命令、不碰敏感目录）；低触发率（用户明确要乐子才出现）；每次 1-2 个玩法。

## 红线执行情况

本包红线已同时写入 `qibai-plugin.json` 的 `system_prompt` 与 `agents/geek.md` 正文：

- **禁 rm -rf / sudo / 危险命令**：system_prompt 第 2 条、工作流程第 4 条、输出规范第 2 条三重禁止，不输出任何真实可执行的修改系统的命令；✅ 已执行。
- **禁写文件/真实系统修改**：只提供输出即结束的展示类玩法，不引导安装执行；✅ 已执行。
- **低触发率**：仅在用户明确说「玩点好玩的/极客玩法/终端彩蛋」时激活，日常对话不主动推荐；✅ 已执行。
- **内容白名单**：fortune 只取温和子集，不引入冒犯类分类；ASCII 文案积极无害；✅ 已执行。
- **终端转义卫生**：不输出会让终端「看起来坏了」的残留转义序列；✅ 已执行。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 硬要求 | 本包实现 |
|---|---|
| `.codebuddy-plugin/plugin.json`（官方隐藏目录） | ✅ |
| `expertType:"agent"` | ✅ |
| `name` 小写连字符 + `plugin` 同值 | ✅ `qibai-geek` |
| `author{name,email}` | ✅ Qibai / qibai@local |
| displayName/profession/displayDescription 双语 | ✅ `{en,zh}` 齐全 |
| displayDescription 中文 40–50 字 | ✅ 已用 python len 校验（见交付回报） |
| tags 固定 3 个（双语） | ✅ 终端彩蛋/ASCII 艺术/趣味玩法 |
| quickPrompts 固定 3 个且首条=defaultInitPrompt | ✅ |
| avatar 512×512 PNG ≤500KB | ✅ |
| categoryId（15 选 1） | ✅ `02-Engineering` |
| agents/*.md frontmatter（name/description 英/displayName/profession 双语/maxTurns:50） | ✅；无 tools 字段（系统统一分配） |

## 权限与安全设计

- 专家本身 permissions 为空，不执行任何 shell——所有「玩法」都是文本演示，不是真实命令。
- system_prompt 三重红线：危险命令零容忍、敏感目录不提、低触发率不打扰主流程。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 纯提示词包 |
| 2 | 无数据外泄指令 | ✅ 无联网彩蛋、无上报 |
| 3 | 无越权系统命令 | ✅ 不执行 shell，且禁止输出危险命令 |
| 4 | 无不安全 API 调用 | ✅ 无技能调用 |
| 5 | 命令注入防护 | ✅ 不执行 shell |
| 6 | 路径遍历防护 | ✅ 明确禁止提及 ~/.ssh 等敏感路径 |
| 7 | 异常输入优雅失败 | ✅ 用户想装工具时只给温和提示，不代执行 |

## 完善点（相对原素材）

- 原 awesome-cli-apps 是纯清单（figlet/fortune/ponysay）→ 重写为对话式专家：每次 1-2 个 + ASCII 演示。
- 新增：危险命令零容忍红线显式写入 system_prompt（原素材最大风险就是把 `sudo rm -rf /` 当彩蛋输出）。
- 新增：低触发率原则（<5% 主动打扰的反向——完全被动激活）。
- 新增：WorkBuddy 双语市场字段、头像、分类、tags/quickPrompts。

## 语音调用示例

- 「元气小小，玩点好玩的，给我来个终端彩蛋」→ 命中 geek 专家
- 「元气小小，给我来一句 fortune 风格的随机语录」
- 「元气小小，用 figlet 大字拼出我的名字」

## 自测结果

- `python3 -c "import json; json.load(open('qibai-plugin.json')); json.load(open('.codebuddy-plugin/plugin.json'))"` ✅
- displayDescription.zh 字数校验 ✅（见交付回报）
- quickPrompts[0] == defaultInitPrompt ✅
- 头像 512×512、≤500KB ✅
- 扫描装配与专家激活由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
