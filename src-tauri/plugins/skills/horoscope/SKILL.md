---
name: horoscope
display_name: 星座运势
display_name_en: Daily Horoscope
description: 查看今日星座运势，本地规则生成，娱乐性质。触发词：星座运势、今日运势、看看白羊座、我的星座。
description_zh: 按星座生成今日运势：总体评分、幸运色、幸运数字、事业爱情健康小贴士，全龄友好、娱乐向。
description_en: Generate a daily horoscope with score, lucky color, lucky number and tips for work, love and health. For entertainment only.
version: 1.0.0
author: Qibai
examples_zh:
  - "使用星座运势，看看白羊座今天怎么样"
  - "帮我看看我这周的狮子座运势"
allowed-tools: Bash
---

# 星座运势（horoscope）

> 版本：1.0.0 ｜ 适配腾讯 WorkBuddy 技能规范 ｜ 来源参考与安全自查见包内 README.md

## Trigger

当用户提到星座、运势、今日运程、幸运色/幸运数字时触发。典型说法：

- 「使用星座运势，看看白羊座今天怎么样」
- 「帮我看看今天的射手座运程」
- 「我的星座今天运势如何」

## Instructions

1. 调用函数 `today_fortune`，参数 `sign` 传用户提到的星座名（如 白羊座）。
2. 用户没说星座时，先询问（列出 12 星座）；脚本返回结果后按原文播报。
3. 脚本结果为确定性随机（同一天同一星座稳定），属娱乐参考：播报时保持轻松语气，
   结尾带上「运势仅供娱乐，积极心态最重要」的温馨提示，不承诺、不绝对化、不引导决策。
4. 用户要求「明日/本周/下周」时，明确告知本技能仅生成当日运势，不编造其他时段结果。

## Example

用户：使用星座运势，看看白羊座今天怎么样

调用：`today_fortune(sign="白羊座")`

回复（示意）：白羊座今日运势：总体 78 分，关键词「灵感闪现」，幸运色天蓝色……（温馨提示后播报）

## Changelog

- 1.0.0（2026-10-24）：首版。本地规则生成：评分/关键词/幸运色/幸运数字/事业爱情健康三栏；确定性随机（日期+星座为种子）；全龄合规、娱乐性质。
