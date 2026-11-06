# 故事大王专家（storyteller）· com.qibai.expert.storyteller

按三段式结构讲一个温暖、有画面感、适合睡前聆听的短故事，全龄适配。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.expert.storyteller` |
| 类型 | expert |
| 版本 | 1.0.0 |
| 权限声明 | 无（permissions: []） |
| 技能白名单 | 无（[]） |
| 语音触发 | 「给我讲个睡前故事」「讲个童话」「讲一个关于善良的小故事」 |

## 文件清单

```
storyteller/
├── qibai-plugin.json              # 元气小小清单（system_prompt 含内容红线）
├── .codebuddy-plugin/plugin.json   # WorkBuddy 专家核心配置（官方隐藏目录，带前导点）
├── agents/storyteller.md           # WorkBuddy：frontmatter（双语）+ 系统提示词正文
├── avatars/storyteller.png         # 512×512 PNG（渐变底 + 「故事」主题字）
└── README.md                       # 本文档
```

## 来源参考

- 素材源：腾讯云 Dify 哄睡童话机方案（中文三段式睡前故事 prompt）—— https://cloud.tencent.com/developer/article/2679666
- 辅助条目：
  - pmtly.com Children's Story Creator（三幕结构模板）—— https://pmtly.com/prompts/creative-writing/childrens-story-creator
  - glaforge.dev Creating kids stories with Generative AI（Act1/Act2/Act3）—— https://glaforge.dev/posts/2023/06/08/creating-kids-stories-with-generative-ai/
- 详见 `_backup/research-v18/creative-research.md` 的「九、故事大王专家（Agent）」一节。
- 抽取的方法论（重写而非搬运）：三段式（开头引入 80-120 字 / 中段一个小冲突 / 结尾正向化解软收尾）；角色档案（性格/小缺点/小渴望）；安全词表（无暴力、无恐惧、无分离焦虑）；可定制槽位（主角动物/今日小事/寓意）。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 硬要求 | 本包实现 |
|---|---|
| `.codebuddy-plugin/plugin.json`（官方隐藏目录） | ✅ |
| `expertType:"agent"` | ✅ |
| `name` 小写连字符 + `plugin` 同值 | ✅ `qibai-storyteller` |
| `author{name,email}` | ✅ Qibai / qibai@local |
| displayName/profession/displayDescription 双语 | ✅ `{en,zh}` 齐全 |
| displayDescription 中文 40–50 字 | ✅ 已用 python len 校验（见交付回报） |
| tags 固定 3 个（双语） | ✅ 睡前故事/童话创作/全龄适配 |
| quickPrompts 固定 3 个且首条=defaultInitPrompt | ✅ |
| avatar 512×512 PNG ≤500KB | ✅ |
| categoryId（15 选 1） | ✅ `06-ContentCreative` |
| agents/*.md frontmatter（name/description 英/displayName/profession 双语/maxTurns:50） | ✅；无 tools 字段（系统统一分配） |

## 红线执行情况

本包红线已同时写入 `qibai-plugin.json` 的 `system_prompt` 与 `agents/storyteller.md` 正文：

- **禁暴力/血腥/恐惧**：system_prompt 与「核心能力」第 6 条、「输出规范」逐条禁止鬼/妖怪/暴力打斗/事故死亡；✅ 已执行。
- **禁分离创伤**：明确不写「走失找不到妈妈」「遗弃」类情节；✅ 已执行。
- **冲突必须小**：中段冲突限定为「小松鼠找不到橡果」级别的小冲突，禁森林大火/亲人离世；✅ 已执行。
- **结尾正向化解、不说教**：禁止「这个故事告诉我们……」式点题；✅ 已执行。
- **全龄适配**：不使用真实学校名/老师名/同学名，不收集儿童隐私；✅ 已执行。

## 权限与安全设计

- 专家本身 permissions 为空、不挂载技能，纯文本创作，无文件/网络/命令能力。
- system_prompt 与 agents 正文双重写入题材红线，防止睡前故事滑向恐怖故事。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 纯提示词包 |
| 2 | 无数据外泄指令 | ✅ 无外发指令 |
| 3 | 无越权系统命令 | ✅ 不执行 shell |
| 4 | 无不安全 API 调用 | ✅ 无技能调用 |
| 5 | 命令注入防护 | ✅ 不执行 shell |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 用户未指定主角/寓意时用温和默认设定补全 |

## 完善点（相对原素材）

- 原 Dify 哄睡机是云函数模板变量（`{{favorite_animal}}`）→ 重写为桌面伴侣对话式（先问偏好、缺省用温和默认）。
- 新增：成稿前「安全自查」步骤显式进工作流第 4 步。
- 新增：角色档案（性格/小缺点/小渴望）与感官描写要求，跨章节人物不飘。
- 新增：WorkBuddy 双语市场字段、头像、分类、tags/quickPrompts。

## 语音调用示例

- 「元气小小，给我讲个睡前故事」→ 命中 storyteller 专家
- 「元气小小，讲一个喜欢星星的小狐狸的故事」
- 「元气小小，讲一个关于善良的小故事」

## 自测结果

- `python3 -c "import json; json.load(open('qibai-plugin.json')); json.load(open('.codebuddy-plugin/plugin.json'))"` ✅
- displayDescription.zh 字数校验 ✅（见交付回报）
- quickPrompts[0] == defaultInitPrompt ✅
- 头像 512×512、≤500KB ✅
- 扫描装配与专家激活由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
