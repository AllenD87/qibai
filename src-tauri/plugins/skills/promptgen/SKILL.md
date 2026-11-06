---
name: promptgen
display_name: 绘画提示词
display_name_en: AI Art Prompt Generator
description: 生成提示词、画图提示词、帮我写个绘画描述：把你说的主体/风格/光照/镜头拼成英文 AI 绘画 prompt + 中文对照。触发词：生成提示词、画图提示词、prompt、绘画描述。
description_zh: 本地关键词映射模板输出英文 prompt（主体+风格+光照+镜头+细节+负面词）加中文对照，中英双语。
description_en: Build an English AI-art prompt (subject, style, lighting, camera, details, negatives) with a Chinese gloss, from a local keyword map.
version: 1.0.0
author: Qibai
examples_zh:
  - "生成提示词，一只橘猫，水彩风，黄昏光"
  - "帮我写个绘画描述，雪山特写"
allowed-tools: Bash
---

# 绘画提示词（promptgen）

> 版本：1.0.0 ｜ 适配腾讯 WorkBuddy 技能规范 ｜ 来源参考与安全自查见包内 README.md

## Trigger

当用户要给 AI 绘画工具（SD/MJ 类）写提示词时触发。典型说法：

- 「生成提示词，一只橘猫，水彩风，黄昏光」
- 「画图提示词怎么写，我想画雪山」
- 「帮我写个绘画描述」
- 「prompt」

## Instructions

1. 调用函数 `generate_prompt`，参数 `subject` 传用户整句描述（如「雪山，水彩风，黄昏光，特写」）。
2. 脚本按「主体→风格→光照→镜头」四段关键词映射拼出英文 prompt，并附中文对照与通用负面词；结果按原文播报。
3. 用户描述太模糊时（如只说「好看的画」），脚本会给通用模板并提示可补充的细节（主体/风格/光照/镜头），主模型可顺势追问一句。
4. **内容边界**：只输出通用流派与虚构主体词，不写真实人名、不写在世艺术家姓名作风格标签、不生成真人色情内容；负面词已内置 `nsfw/nude/naked/sexual/weapon/blood/gore`。用户要画真人写真/名人脸时，提示改用虚构主体描述。

## Example

用户：帮我写个绘画描述——雪山，水彩风，黄昏光，特写

调用：`generate_prompt(subject="雪山，水彩风，黄昏光，特写")`

回复（示意）：English prompt: misty snow mountains with pine trees, watercolor painting, warm golden hour lighting, close-up shot, masterpiece, best quality… 中文对照：主体=雪山松林 ｜ 风格=水彩画 ｜ 光照=黄昏金色暖光 ｜ 镜头=特写镜头。

## Changelog

- 1.0.0（2026-10-24）：首版。主体/风格/光照/镜头四槽关键词中英映射，固定模板输出英文 prompt + 负面词 + 中文对照；负面词含 nsfw/暴力词屏蔽；零权限纯本地。
