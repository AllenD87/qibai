---
name: translate
display_name: 翻译助手
display_name_en: Translator
description: 中英互译：脚本查本地常用词表给参考，主模型整句润色。触发词：翻译、翻译一下、翻成英文、翻成中文、这个用英语怎么说。
description_zh: 用户给出中文或英文文本时，先由脚本查本地常用中英对照表输出方向/词表译文/术语参考，再由主模型完成整句翻译与润色；脚本只报告不改写。
description_en: Translate between Chinese and English. The script looks up a local phrase table for reference; the main model renders the whole sentence. Local only, no network.
version: 1.0.0
author: Qibai
examples_zh:
  - "翻译一下：你好"
  - "这个用英语怎么说：我稍后回复你"
allowed-tools: Bash
---

# 翻译助手（translate）

> 版本：1.0.0 ｜ 适配腾讯 WorkBuddy 技能规范 ｜ 来源参考与安全自查见包内 README.md

## Trigger

当用户要求做中英互译，或询问某个词/句子的另一种说法时触发。典型说法：

- 「翻译一下：你好」
- 「把这句话翻成英文：会议几点开始」
- 「翻成中文：as soon as possible」
- 「这个用英语怎么说：我稍后回复你」
- 「帮忙翻译一下这封邮件的大意」

## Instructions

1. 调用函数 `translate_text`，参数 `text` 传用户要翻译的原文（原样保留，不要改写）。
2. 脚本做确定性处理，分工如下：
   - 自动识别方向（中→英 / 英→中）；
   - 整句命中本地词表时直接给出参考译文；未命中时**不做子串自动替换**，只列出被原文包含的术语词条供核对；
   - 脚本是「查表与校验」角色，**只报告、不改写**，整句译文由你完成。
3. 收到脚本结果后：
   - 整句已命中词表：可直接播报译文，并按语境微调语气；
   - 未命中：由你整句翻译，译文中的术语必须与脚本给出的「术语参考」保持一致；
   - 英文→中文时：译文里英文与中文之间保留半角空格，中文语境用全角标点。
4. 专业/法律/医疗等敏感文本，结尾提示用户「译文仅供参考，重要文件请人工复核」。
5. 用户要求翻译成第三语言（日语等）时，说明本技能目前只做中英互译，不编造其他语言结果。

## Example

用户：这个用英语怎么说：我稍后回复你

调用：`translate_text(text="我稍后回复你")`

脚本返回（示意）：方向 中→英，整句命中 `I will get back to you later`

回复（示意）：这句话可以说：「I'll get back to you later.」口语里也常说「I'll follow up with you soon.」

## Changelog

- 1.0.0（2026-10-24）：首版。本地 60 条常用中英对照表（整句精确命中 + 术语参考列出）；整句翻译归主模型，脚本只查表校验、不子串替换；纯本地无网络。
