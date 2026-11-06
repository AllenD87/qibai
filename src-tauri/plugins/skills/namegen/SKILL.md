---
name: namegen
display_name: 创意命名
display_name_en: Creative Name Generator
description: 起个名字、取个名、帮我命名、给产品起名：科技/文艺/萌宠/游戏/国风五组词根组合出 5 个候选名+注解。触发词：起个名字、取个名、帮我命名、给产品起名。
description_zh: 本地词根库按场景随机组合 5 个候选名，每个带简短寓意注解；只讲音韵寓意，不碰八字五格。
description_en: Combine local root-word banks into 5 candidate names per scene (tech/arts/pet/game/guofeng), each with a short meaning note.
version: 1.0.0
author: Qibai
examples_zh:
  - "起个名字，要萌宠风"
  - "帮我给产品命名"
allowed-tools: Bash
---

# 创意命名（namegen）

> 版本：1.0.0 ｜ 适配腾讯 WorkBuddy 技能规范 ｜ 来源参考与安全自查见包内 README.md

## Trigger

当用户要给产品/项目/宠物/角色起名字时触发。典型说法：

- 「起个名字，要萌宠风」
- 「取个名，文艺一点的」
- 「帮我给这个产品命名」
- 「给猫/给宝宝起个名」

## Instructions

1. 调用函数 `generate_names`，参数 `scene` 传用户想要的风格（科技/文艺/萌宠/游戏/国风；用户没说时不传，脚本随机抽一种并说明）。
2. 脚本输出 5 个两字候选名 + 每个的词根寓意注解，按原文播报。
3. 用户想换一批时直接再问一次（随机种子不同会出新组合）；想调整字数/性别/调性时，主模型可引导用户说明，脚本固定输出两字名。
4. **内容边界**：注解只讲「寓意/音韵/好记」，**不做八字、三才五格、五行算命类解读**；播报时提醒「正式使用前自行核查商标与域名」，不拍胸脯说「这个名字肯定没被注册」。

## Example

用户：起个名字，要萌宠风

调用：`generate_names(scene="萌宠")`

回复（示意）：【创意命名 · 萌宠风】1. 「奶叽」——「奶」软嫩入口，「叽」叽叽喳喳……（共 5 个，末尾带商标自查提示）

## Changelog

- 1.0.0（2026-10-24）：首版。五场景（科技/文艺/萌宠/游戏/国风）各 8 前缀×8 后缀词根库，确定性偏移组合出 5 个不重复候选名 + 逐字寓意注解；不含迷信内容；零权限纯本地。
