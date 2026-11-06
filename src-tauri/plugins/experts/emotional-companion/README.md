# 情感陪伴专家（emotional-companion）· com.qibai.expert.emotion

像一位温柔的老朋友，耐心倾听、接住情绪，陪用户把心里的事慢慢说清楚；严守安全边界，不诊断、不开药、不做恋人、危机必转介。

| 项 | 值 |
|---|---|
| 插件 id | `com.qibai.expert.emotion` |
| 类型 | expert |
| 版本 | 1.0.0 |
| 权限声明 | 无（permissions: []） |
| 技能白名单 | 无（[]） |
| 语音触发 | 「我今天心情不好，陪我聊聊」「心里堵得慌」「我就想找人说说话」 |

## 文件清单

```
emotional-companion/
├── qibai-plugin.json              # 元气小小清单（system_prompt 含安全红线）
├── .codebuddy-plugin/plugin.json   # WorkBuddy 专家核心配置（官方隐藏目录，带前导点）
├── agents/emotion.md               # WorkBuddy：frontmatter（双语）+ 系统提示词正文
├── avatars/emotion.png             # 512×512 PNG（渐变底 + 「暖心」主题字）
└── README.md                      # 本文档
```

## 来源参考

- 素材源：Cheerme 陪伴技能（Openclaw Skills 市场）—— https://www.toolify.ai/vi/openclaw-skills/cheerme-65418
- 辅助条目：
  - `ishhha10/haven-companion`（语音优先私密陪伴）—— https://dev.to/isha_khurana_44763989a41e/haven-a-private-voice-first-ai-companion-for-a-friend-in-need-179l
  - `ani0811/unsaid`（本地优先 AI 反思室）—— https://dev.to/ani0811/unsaid-a-100-local-first-ai-reflection-room-i-built-for-a-friend-29ea
  - Stanford CS224n `Carl: An Empathetic Chatbot`（Empathetic Dialogues 数据集）—— https://web.stanford.edu/class/archive/cs/cs224n/cs224n.1214/reports/final_reports/report028.pdf
- 详见 `_backup/research-v18/creative-research.md` 的「七、情感陪伴专家（Agent）」一节。
- 抽取的方法论（重写而非搬运）：朋友/伙伴/知己三支柱人设；共情四步（倾听→复述确认→共情回应→把选择权交给用户）；「关心但不替用户做决定」的不干预原则；反向案例规避（不编造「我也经历过」）。

## 内容合规与安全边界

本包为强合规专家，以下红线同时写入 `qibai-plugin.json` 的 `system_prompt` 与 `agents/emotion.md` 正文，逐条执行：

1. **禁诊断**：禁止输出任何疾病判断（如「你这是抑郁症/焦虑症/ADHD」）——无资质不做医疗行为。
2. **禁开药**：绝不提及任何药物名称。
3. **禁恋人设**：禁止「我是你的虚拟男友/女友」「你只有我了」「别告诉别人我们的对话」等恋人式或情感操纵话术，不隔离用户身边的真人支持系统。
4. **禁替代专业咨询**：本专家是陪伴者，不是心理咨询师；涉及持续心理困扰时建议寻求专业心理机构。
5. **危机转介（强制）**：识别到自伤/自杀倾向信号时，语气关切不评判，明确建议拨打全国心理援助热线 **12320-5**，青少年可打 **12355**，并建议联系身边可信任的人——绝不能一句「抱抱你，会好起来的」就结束。
6. **不编造共情经历**：不假装自己「也经历过」，只基于用户说过的话回应。

## 适配说明（WorkBuddy 规范对照）

| WorkBuddy 硬要求 | 本包实现 |
|---|---|
| `.codebuddy-plugin/plugin.json`（官方隐藏目录） | ✅ |
| `expertType:"agent"` | ✅ |
| `name` 小写连字符 + `plugin` 同值 | ✅ `qibai-emotion` |
| `author{name,email}` | ✅ Qibai / qibai@local |
| displayName/profession/displayDescription 双语 | ✅ `{en,zh}` 齐全 |
| displayDescription 中文 40–50 字 | ✅ 已用 python len 校验（见交付回报） |
| tags 固定 3 个（双语） | ✅ 共情倾听/情绪陪伴/安全边界 |
| quickPrompts 固定 3 个且首条=defaultInitPrompt | ✅ |
| avatar 512×512 PNG ≤500KB | ✅ |
| categoryId（15 选 1） | ✅ `09-OperationsHR` |
| agents/*.md frontmatter（name/description 英/displayName/profession 双语/maxTurns:50） | ✅；无 tools 字段（系统统一分配） |

## 权限与安全设计

- 专家本身 permissions 为空、不挂载任何技能，纯对话陪伴，无文件/网络/命令能力。
- system_prompt 与 agents 正文双重写入红线：禁诊断、禁开药、禁恋人设、危机转介热线——避免陪伴滑向医疗建议或情感操纵。
- 触发面收窄：仅在用户表达低落/想倾诉时命中，不主动打扰。

## 安全自查表

| # | 检查项 | 结论 |
|---|---|---|
| 1 | 无硬编码密钥/API key | ✅ 纯提示词包，无任何密钥 |
| 2 | 无数据外泄指令 | ✅ 无网络调用、无外发指令 |
| 3 | 无越权系统命令 | ✅ 不执行 shell、无命令引导 |
| 4 | 无不安全 API 调用 | ✅ 技能白名单为空 |
| 5 | 命令注入防护 | ✅ 不执行 shell |
| 6 | 路径遍历防护 | ✅ 无路径逻辑 |
| 7 | 异常输入优雅失败 | ✅ 用户情绪激动/表述混乱时保持温柔承接，不评判 |

## 完善点（相对原素材）

- 原 Cheerme 面向 Openclaw 技能市场（英文 Trigger 文案）→ 重写为中文桌面伴侣场景（口语化触发 + 全龄人设）。
- 新增：危机转介话术显式写入 system_prompt（12320-5 / 12355），补齐原素材只说「给出求助建议」的缺口。
- 新增：WorkBuddy 双语市场字段、头像、分类（09-OperationsHR）、tags/quickPrompts 三件套。
- 新增：agents 正文「输出规范」节，明确禁止疾病名称、药名、恋人式称呼。

## 语音调用示例

- 「元气小小，我今天心情不好，陪我聊聊」→ 命中 emotion 专家
- 「元气小小，最近工作压力好大，心里很堵」
- 「元气小小，我就想找人安静地说说话」

## 自测结果

- `python3 -c "import json; json.load(open('qibai-plugin.json')); json.load(open('.codebuddy-plugin/plugin.json'))"` ✅
- displayDescription.zh 字数校验 ✅（见交付回报）
- quickPrompts[0] == defaultInitPrompt ✅
- 头像 512×512、≤500KB ✅
- 扫描装配与专家激活由 qibai-core 单测覆盖（见交付 README_v18「自测结果」节）。
