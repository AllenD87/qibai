---
name: file-search
display_name: 文件搜索
display_name_en: File Search
description: 在用户主目录内按文件名关键词只读查找文件。触发词：找文件、搜索文件、文件在哪里、帮我找一下。
description_zh: 用文件名关键词（兼容中英文）在用户目录下模糊查找文件，自动排除依赖目录与敏感目录，搜不到明说、不编造路径。
description_en: Read-only filename search inside the user home directory, with sensitive-directory exclusions and honest empty results.
version: 1.0.0
author: Qibai
examples_zh:
  - "帮我找一下「简历」这个文件"
  - "找文件，关键词 report"
allowed-tools: Bash
---

# 文件搜索（file-search）

> 版本：1.0.0 ｜ 适配腾讯 WorkBuddy 技能规范 ｜ 来源参考与安全自查见包内 README.md

## Trigger

当用户提到找文件、搜文件、定位文档，给出一个文件名关键词时触发。典型说法：

- 「帮我找一下简历那个文件」
- 「搜索文件，关键词 report」
- 「我上次保存的发票在哪里？」

## Instructions

1. 调用函数 `search_files`，参数 `keyword` 传用户原话里的文件名关键词（两三个字即可，不要改写、不要补全路径）。
2. 脚本在 `$HOME` 下最深 6 层按文件名模糊匹配，自动排除 node_modules/.git/target/.cache、.ssh 等隐私目录与含 secret/password 字样的目录。
3. 脚本输出两类结果，分别处理：
   - **命中**：把路径列表回读给用户（逐条念出即可），并提醒「这是文件名匹配结果」；
   - **未找到**：按脚本提示建议用户换更短的关键词或说明文件类型；**绝不脑补、绝不编造一个看似合理的路径**。
4. 用户给的是一段描述而非文件名（如「我上周写的方案」）时，先从中提炼关键词（如「方案」）再调用；提炼不出关键词时请用户补充。
5. 本技能只按文件名查找（Glob 通道），不读取文件内容、不汇报文件大小以外的信息；用户要看内容时提示另行操作。

## Example

用户：帮我找一下「发票」这个文件

调用：`search_files(keyword="发票")`

返回（示意）：`【文件搜索】……找到这些文件名包含「发票」的文件：/home/user/Documents/发票-10月.pdf ……`

回复：帮你找到 2 个文件名里带「发票」的文件：/home/user/Documents/发票-10月.pdf、/home/user/Downloads/发票照片.jpg。全程只读，没有改动任何文件。

## Changelog

- 1.0.0（2026-10-24）：首版。Glob 文件名通道只读查找；限定 $HOME、maxdepth 6；默认排除依赖目录与敏感目录；`..`/超长/空参友好提示；搜不到明说、不编造路径。
