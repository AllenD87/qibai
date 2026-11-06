# code-writer 技能包说明

- 适配：腾讯 WorkBuddy 技能规范（manifest + SKILL.md + run.sh 入口）
- 触发：「帮我写」「写一个」「生成代码」「写个脚本」
- 能力：HTML / Python / JavaScript / Shell / JSON / Rust / Go / CSS 代码骨架生成
- 交付形态：面板独立窗展示（深色代码块，可复制）+ 语音摘要
- 安全：只读文本输出；需求不拼 shell 命令；无文件读写；无网络；无密钥
- 完善点：模板为高频骨架，复杂定制由主对话 LLM 生成完整实现；新增语言模板可在此 run.sh 中按现有模式扩展（增加识别 case + CODE 模板 + 面板 title）
