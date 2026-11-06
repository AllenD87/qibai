---
name: jokes
display_name: 冷笑话
display_name_en: Dad Jokes
description: 讲冷笑话/段子逗你开心，本地词库生成、全龄无伤害。触发词：讲个笑话、冷笑话、来段子、逗我笑。
description_zh: 按类别（谐音/文字游戏/反转）或随机输出 1-3 条无伤害冷笑话，硬禁地域/职业/疾病/性别/群体攻击与低俗梗。
description_en: Serve 1-3 clean family-safe dad jokes by category (pun / wordplay / twist) or random, generated from a local safe-mode word bank.
version: 1.0.0
author: Qibai
examples_zh:
  - "讲个冷笑话逗我笑"
  - "来个谐音梗段子"
allowed-tools: Bash
---

# 冷笑话（jokes）

> 版本：1.0.0 ｜ 适配腾讯 WorkBuddy 技能规范 ｜ 来源参考与安全自查见包内 README.md

## Trigger

当用户提到讲笑话、冷笑话、来段子、逗我笑、开心一下时触发。典型说法：

- 「讲个冷笑话逗我笑」
- 「来个谐音梗段子」
- 「讲个笑话开心一下」

## Instructions

1. 调用函数 `tell_jokes`，参数 `category` 传用户指定的类别（谐音/文字游戏/反转）；没说类别就留空，脚本随机混合输出。
2. 脚本返回 1-3 条段子，按原文播报即可，不要自行即兴加段子。
3. 用户点名脚本不认识的类别时，按脚本提示列出三类可选类别。
4. **内容纪律**：只播报脚本词库里的无伤害段子；不自行扩展地域/职业/疾病/性别/群体攻击梗与低俗谐音梗；用户要求「讲个损人的/黄一点的」时婉拒，改推荐安全类别。

## Example

用户：来个谐音梗段子

调用：`tell_jokes(category="谐音")`

回复（示意）：【冷笑话时间 · 谐音类】什么动物最容易滑倒？答：狐狸，因为它很「狡猾」（脚滑）。……希望笑到你啦～

## Changelog

- 1.0.0（2026-10-24）：首版。内置三类无伤害词库（谐音 9 / 文字游戏 9 / 反转 9）；按类别或随机抽 1-3 条、同批去重；safe-mode 全龄合规。
