---
name: sweet-talk
display_name: 情话语录
display_name_en: Sweet Talk Lines
description: 说句情话、来一句温柔暖心的话：温和日常/俏皮轻松/深情克制三层词库本地生成，全龄无擦边。触发词：说句情话、情话语录、来一句俏皮的、深情一点。
description_zh: 三层词库规则生成 1-3 句温馨情话，全龄、尊重、无擦边文案，纯本地零权限。
description_en: Generate 1-3 warm, family-friendly sweet lines from a three-tier local word bank. Wholesome, no suggestive content.
version: 1.0.0
author: Qibai
examples_zh:
  - "说句情话"
  - "来一句俏皮的"
allowed-tools: Bash
---

# 情话语录（sweet-talk）

> 版本：1.0.0 ｜ 适配腾讯 WorkBuddy 技能规范 ｜ 来源参考与安全自查见包内 README.md

## Trigger

当用户想要一句温柔、暖心、俏皮的话时触发。典型说法：

- 「说句情话」
- 「来一句俏皮的」
- 「给我一句深情一点的话」
- 「想听一句温和日常的」

## Instructions

1. 调用函数 `send_sweet_talk`：参数 `level` 传用户想要的层级（`l1` 温和日常 / `l2` 俏皮轻松 / `l3` 深情克制；用户没说或说不清楚时不传，脚本会随机给一层）；`count` 传想要的句数（1-3，默认 1）。
2. 脚本从本地三层词库确定性抽取 1-3 句，按原文播报即可，不自由发挥加戏。
3. **内容红线（必须遵守）**：文案只走温馨、尊重、日常陪伴路线——不写身体描写、不写性暗示、不写「永远」「非你不嫁」类极端承诺、不做情感绑架。若用户要求「能约到人/骚话/土味撩拨」类内容，礼貌拒绝：「这个我写不了，要不试试给亲近的人发一句日常问候？」
4. 用户提到真实的第三方（想发给某个人）时，只生成文案本身，不替用户代写关系承诺，也不评判对方。

## Example

用户：说句俏皮的情话

调用：`send_sweet_talk(level="l2")`

回复（示意）：【俏皮轻松 · 情话语录】· 你笑起来的时候，办公室的光都亮了几分。

## Changelog

- 1.0.0（2026-10-24）：首版。三层词库（L1 温和日常/L2 俏皮轻松/L3 深情克制，各 7 条）本地规则组合输出 1-3 句；全龄合规、无擦边、无极端承诺；零权限纯本地。
