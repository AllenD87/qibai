---
name: reminder-parse
display_name: 待办解析
display_name_en: Reminder Parser
description: 把自然语言待办/提醒文本解析为结构化 JSON。触发词：待办解析、提醒我、记一下、别忘了、安排提醒。
description_zh: 解析用户原话中的时间与事项（今天/明天/几点/周几等），输出结构化待办 JSON，配合提醒功能使用。
description_en: Parse natural-language todo and reminder text into structured JSON for scheduling.
version: 1.0.0
author: Qibai
examples_zh:
  - "明天下午3点开会提醒我"
  - "帮我记一下：周五晚上8点健身"
allowed-tools: Bash
---

# 待办解析（reminder-parse）

> 版本：1.0.0 ｜ 适配腾讯 WorkBuddy 技能规范 ｜ 来源参考与安全自查见包内 README.md

## Trigger

当用户提到待办、提醒、日程安排，且带有时间表达（今天/明天/几点/周几等）时触发。典型说法：

- 「明天下午3点开会，提醒我」
- 「使用待办解析：后天上午10点交周报」
- 「帮我记一下：周五晚上8点健身」

## Instructions

1. 调用函数 `parse_reminder`，参数 `text` 传用户原话（完整保留，不要改写）。
2. 脚本返回结构化 JSON：`ok / raw / date / time / when_iso / what`。
   - `ok=false`：解析失败，按 `error` 提示用户补充信息；
   - `ok=true`：把结果回读给用户确认（如「好的，我记下了：10月25日 15:00 开会」），
     然后调用内置提醒功能（`set_reminder`）设置提醒。
3. 时间未指定时默认今天；时间只有时段（下午/晚上）时使用脚本的默认钟点；事项缺失时请追问。
4. 涉及跨天/跨周表达（下周一等）时，以脚本计算出的 `date` 为准，不必自行推算。
5. 设置提醒前把解析结果与用户核对一遍，避免误设。

## Example

用户：明天下午3点开会提醒我

调用：`parse_reminder(text="明天下午3点开会提醒我")`

返回（示意）：`{"ok":true,"date":"2026-10-25","time":"15:00","when_iso":"2026-10-25T15:00:00","what":"开会"}`

回复：好的，明天（10月25日）下午 15:00 提醒你「开会」。然后调用 set_reminder 设置提醒。

## Changelog

- 1.0.0（2026-10-24）：首版。支持今天/明天/后天/大后天、本周X/下周X、HH:MM/X点/X点半/X点Y分、上午/下午/晚上等表达；输出结构化 JSON；只读无副作用。
