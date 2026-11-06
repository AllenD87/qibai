---
name: code-review
display_name: 代码审查
display_name_en: Code Review
description: 只读扫描代码目录，找出 eval/硬编码密钥/rm -rf 等高危模式，带文件:行号和修复建议。触发词：审查这段代码、检查代码、代码审查、有没有安全隐患。
description_zh: 对用户指定的本地代码目录做只读静态扫描，命中危险模式清单时输出「文件:行号 + 风险类型 + 修复建议」，未命中报告未发现；只读，不修改任何文件。
description_en: Read-only static scan of a local code directory against a danger-pattern list (eval, hardcoded secrets, rm -rf, unsafe SQL, curl|sh). Reports file:line plus fix hints.
version: 1.0.0
author: Qibai
examples_zh:
  - "审查这段代码：projects/my-app"
  - "帮我看看这个项目有没有安全隐患"
allowed-tools: Bash
---

# 代码审查（code-review）

> 版本：1.0.0 ｜ 适配腾讯 WorkBuddy 技能规范 ｜ 来源参考与安全自查见包内 README.md

## Trigger

当用户要求审查/检查代码，或担心代码里有安全隐患时触发。典型说法：

- 「审查这段代码：projects/my-app」
- 「帮我检查一下这个项目的代码」
- 「代码审查一下，有没有安全隐患」
- 「这段脚本会不会把密钥写死在里面」

## Instructions

1. 调用函数 `code_review_scan`，参数 `path` 传用户指定的代码目录（相对路径按 `$HOME` 解析；绝对路径必须在 `$HOME` 内；含 `..` 或越界会被脚本拒绝）。
2. 脚本做确定性只读扫描（危险模式清单：`eval(`、`os.system`/`shell=True`/`exec(`、`rm -rf`、硬编码密钥 `sk-xxx`/`AKIA`/`password="明文"`、无 `WHERE` 的 DELETE/DROP、`curl|sh` 管道），排除 node_modules/.git/venv 等依赖目录。
3. 收到脚本输出后按三级分流处理 findings：
   - **Critical（必须立即修）**：硬编码密钥、`rm -rf`、`eval(` 用户输入；
   - **Important（继续前修）**：命令拼接、`curl|sh` 管道、无 WHERE 的 DELETE/DROP；
   - **Minor（记录后补）**：可讨论的风格项。
   每条 findings 必须保留脚本给出的「文件:行号」，并把「修复建议」用人话展开一两句，不空泛点评。
4. 脚本输出「未发现高危模式」时，如实告知，并可补一句「本次仅扫危险模式清单，不覆盖逻辑 Bug 与性能问题」。
5. 全程只读：不要替用户改文件、不要写入任何内容；需要修复时给出修改建议文本即可。

## Example

用户：帮我看看这个项目有没有安全隐患：projects/my-app

调用：`code_review_scan(path="projects/my-app")`

脚本返回（示意）：🔴 [硬编码密钥/口令] `projects/my-app/config.py:3: API_KEY = "sk-abc123def456"` —— 修复建议：移出代码，改读环境变量并轮换该 key。

回复（示意）：扫到 1 处高危：config.py 第 3 行写死了 sk- 开头的密钥，建议立即移到环境变量并轮换。其余模式未命中。

## Changelog

- 1.0.0（2026-10-24）：首版。只读静态扫描 6 类危险模式，输出 文件:行号 + 风险类型 + 修复建议；路径校验（拒 `..`、限 `$HOME`）；问题三级分流。
