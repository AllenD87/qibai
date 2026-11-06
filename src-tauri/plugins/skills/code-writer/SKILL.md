---
name: code-writer
display_name: 写代码
display_name_en: Code Writer
description: 生成可直接运行的代码骨架/模板。触发词：帮我写代码、写一个 XX、生成代码、写个脚本、帮我写个函数。支持 HTML/Python/JavaScript/Shell/JSON/Rust/Go/CSS；复杂定制（接口对接/业务逻辑/多文件工程）由主对话大模型生成完整实现。
description_zh: 识别用户需求中的语言与用途，输出高质量可运行代码骨架（含安全写法：Python argparse+错误处理、Shell set -eu+全引号、HTML 视口适配等），面板独立窗展示可直接复制；不读写任何文件，纯文本输出。
description_en: Detect the requested language (HTML/Python/JS/Shell/JSON/Rust/Go/CSS) and emit a runnable, security-aware code skeleton, shown in a copyable panel window. Pure text output; reads and writes no files.
version: 1.0.0
author: Qibai
examples_zh:
  - "帮我写一个 Python 脚本读取 CSV"
  - "写一个 HTML 欢迎页面"
  - "帮我写一个 shell 备份脚本"
allowed-tools: Bash
---

# 写代码（code-writer）

> 版本：1.0.0 ｜ 适配腾讯 WorkBuddy 技能规范 ｜ 安全自查见包内 README.md

## Trigger

用户要求生成/编写代码时触发。典型说法：

- 「帮我写一个 Python 脚本读取 CSV」
- 「写一个 HTML 欢迎页面」
- 「帮我写个 shell 备份脚本」
- 「生成一个 JSON 配置文件」

## 行为

1. 解析需求文本，识别目标语言（HTML/CSS/JS/Python/Shell/JSON/Rust/Go）；
2. 命中语言 → 输出对应高质量代码骨架（模板内置安全写法与最佳实践）+ 面板 window 展示（代码已转义，可复制）；
3. 未命中 → 提示可用语言清单与规范示例；
4. 复杂定制代码（业务逻辑/接口对接/多文件工程）→ 引导用户直接对元气小小说「帮我写……」，由主对话大模型生成完整实现（本技能只做高频模板）。

## 安全设计

- 六道安全闸：无命令注入（需求只做匹配与回显）、无路径读写、不联网、无越权、无密钥、异常输入友好提示；
- 面板 HTML 转义 `& < >`，脚本内容安全展示（sane 协议要求）；
- 输出代码模板本身采用安全写法（Shell `set -eu`+引号、Python 异常处理、HTML 视口适配），不含任何密钥/内网地址。

## 面板协议

- 命中语言时输出 `@qibai-panel:begin/end` 块（mode=window, title=「<语言> · 元气小小写代码」, html 含 `<pre>` 代码区，样式深色底便于阅读）；
- 未命中时纯文本提示，不输出面板块。
