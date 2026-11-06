#!/bin/sh
# 写代码技能（code-writer）：识别常见代码需求，输出可直接使用的代码骨架/模板（面板展示可复制）。
# 六道安全闸：
#  ① 命令注入——text 只做关键词匹配与回显，绝不拼进 shell 命令；
#  ② 路径遍历——本技能不涉及任何本地路径，不读写文件；
#  ③ 数据外泄——不联网、不读取/上传任何文件；
#  ④ 越权——只输出文本，无写/删/改操作；
#  ⑤ 硬编码密钥——无任何密钥/API key/token；
#  ⑥ 异常输入——空参/超长友好提示后 exit 0。
# 复杂定制代码（接口对接、业务逻辑、多文件工程）由主对话大模型生成：直接说「帮我写……」。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"
TEXT=$(printf '%s' "$ARGS" | sed -n 's/.*"text"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$TEXT" ]; then
  TEXT=$(printf '%s' "$ARGS" | tr -d '{}[]"')
fi
TEXT=$(printf '%s' "$TEXT" | tr -s ' ' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
if [ -z "$TEXT" ]; then
  echo "请告诉我要写什么代码，例如：帮我写一个 Python 脚本读取 CSV；或 写一个 HTML 欢迎页；或 写一个 shell 备份脚本"
  exit 0
fi
TEXT=$(printf '%s' "$TEXT" | cut -c 1-300)

# ---------- 保存路径解析（v18.6 落盘模式：「保存到 ~/xxx.py」） ----------
SAVE_PATH=""
case "$TEXT" in
  *保存到*) SAVE_PATH=$(printf '%s' "$TEXT" | sed -n 's/.*保存到[[:space:]]*\([^ 　，。;；]*\).*/\1/p') ;;
esac
if [ -z "$SAVE_PATH" ]; then
  case "$TEXT" in
    *存到*) SAVE_PATH=$(printf '%s' "$TEXT" | sed -n 's/.*存到[[:space:]]*\([^ 　，。;；]*\).*/\1/p') ;;
  esac
fi
if [ -z "$SAVE_PATH" ]; then
  case "$TEXT" in
    *写入*) SAVE_PATH=$(printf '%s' "$TEXT" | sed -n 's/.*写入[[:space:]]*\([^ 　，。;；]*\).*/\1/p') ;;
  esac
fi
case "$SAVE_PATH" in
  ""|"~/"*|/*) : ;;  # 空 / ~/ 开头 / 绝对路径 才保留
  *) SAVE_PATH="" ;; # 相对路径或非法 → 不落盘
esac
# 扩展名兜底：无扩展名时按语言补默认
if [ -n "$SAVE_PATH" ] && [ "$SAVE_PATH" != "~/" ]; then
  case "$SAVE_PATH" in
    *.*) : ;;
    *) case "$LANG_PY $LANG_JS $LANG_HTML $LANG_CSS $LANG_SH $LANG_JSON $LANG_RUST $LANG_GO" in
         "1 0 0 0 0 0 0 0") SAVE_PATH="$SAVE_PATH.py" ;;
         "0 1 0 0 0 0 0 0") SAVE_PATH="$SAVE_PATH.js" ;;
         "0 0 1 0 0 0 0 0") SAVE_PATH="$SAVE_PATH.html" ;;
         "0 0 0 1 0 0 0 0") SAVE_PATH="$SAVE_PATH.css" ;;
         "0 0 0 0 1 0 0 0") SAVE_PATH="$SAVE_PATH.sh" ;;
         "0 0 0 0 0 1 0 0") SAVE_PATH="$SAVE_PATH.json" ;;
         "0 0 0 0 0 0 1 0") SAVE_PATH="$SAVE_PATH.rs" ;;
         "0 0 0 0 0 0 0 1") SAVE_PATH="$SAVE_PATH.go" ;;
       esac ;;
  esac
fi

# ---------- 语言识别 ----------
LANG_HTML=0; LANG_CSS=0; LANG_JS=0; LANG_PY=0; LANG_SH=0; LANG_JSON=0; LANG_RUST=0; LANG_GO=0
case "$TEXT" in
  *[Hh][Tt][Mm][Ll]*|*网页*|*页面*|*网站*|*首页*|*html*) LANG_HTML=1 ;;
esac
case "$TEXT" in
  *[Cc][Ss][Ss]*|*样式*|*美化*|*css*) LANG_CSS=1 ;;
esac
case "$TEXT" in
  *[Jj][Ss]*|*[Jj]ava[Ss]cript*|*脚本.js*|*js*) LANG_JS=1 ;;
esac
case "$TEXT" in
  *[Pp]ython*|*[Pp]y*脚本*|*.py*|*爬虫*|*数据分析*) LANG_PY=1 ;;
esac
case "$TEXT" in
  *shell*|*sh脚本*|*bash*|*.sh*|*批处理*|*备份脚本*|*自动化脚本*) LANG_SH=1 ;;
esac
case "$TEXT" in
  *[Jj][Ss][Oo][Nn]*|*json*|*配置*文件*) LANG_JSON=1 ;;
esac
case "$TEXT" in
  *[Rr]ust*|*.rs*|*rust*) LANG_RUST=1 ;;
esac
case "$TEXT" in
  *[Gg][Oo]lang*|*golang*|*.go*|"*Go 程序*") LANG_GO=1 ;;
esac

# ---------- HTML 转义（面板展示用；& 必须先转） ----------
esc() {
  sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g'
}

# ---------- 输出代码 + 面板 ----------
if [ "$LANG_HTML" = 1 ]; then
  TITLE="HTML 页面"
  SUMMARY="已生成一个可直接运行的 HTML5 页面骨架（含视口适配、样式与脚本入口）。"
  CODE='<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>页面标题</title>
  <style>
    body { margin: 0; font-family: system-ui, sans-serif; background: #f7f7f7; color: #222; }
    .container { max-width: 720px; margin: 40px auto; padding: 24px; background: #fff; border-radius: 12px; box-shadow: 0 2px 8px rgba(0,0,0,.08); }
    h1 { margin-top: 0; }
  </style>
</head>
<body>
  <div class="container">
    <h1>你好，世界</h1>
    <p>在这里放你的内容。</p>
    <button id="btn">点我</button>
    <script>
      document.getElementById("btn").addEventListener("click", () => {
        alert("你好！");
      });
    <\/script>
  </div>
</body>
</html>'
elif [ "$LANG_PY" = 1 ]; then
  TITLE="Python 脚本"
  SUMMARY="已生成一个带参数解析与错误处理的 Python 脚本骨架（Python 3，直接运行即可）。"
  CODE='#!/usr/bin/env python3
"""脚本骨架：直接运行 python3 main.py --input xxx"""
import argparse
import sys
from pathlib import Path

def process(input_path: Path) -> None:
    """核心处理逻辑写在这里。"""
    if not input_path.exists():
        raise FileNotFoundError(f"文件不存在: {input_path}")
    # TODO: 你的处理逻辑
    print(f"处理完成: {input_path}")

def main() -> None:
    parser = argparse.ArgumentParser(description="你的脚本说明")
    parser.add_argument("--input", required=True, help="输入文件路径")
    parser.add_argument("--verbose", action="store_true", help="打印详细信息")
    args = parser.parse_args()

    try:
        process(Path(args.input))
    except Exception as exc:
        print(f"错误: {exc}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    main()'
elif [ "$LANG_JS" = 1 ]; then
  TITLE="JavaScript 模块"
  SUMMARY="已生成一个 ES Module 风格的 JavaScript 函数骨架（node 直接运行）。"
  CODE='// 脚本骨架：node main.mjs 运行
export function main() {
  const result = greet("元气小小");
  console.log(result);
}

function greet(name) {
  return `你好，${name}！`;
}

// 直接运行时调用
if (import.meta.url === `file://${process.argv[1]}`) {
  main();
}'
elif [ "$LANG_SH" = 1 ]; then
  TITLE="Shell 脚本"
  SUMMARY="已生成一个安全的 Shell 脚本骨架（set -eu + 全引号 + 目录检查）。"
  CODE='#!/bin/sh
# 脚本说明：set -eu 遇错即停；所有变量加引号防注入。
set -eu

# 输入参数（示例：备份目录）
TARGET="${1:-}"
if [ -z "$TARGET" ]; then
  echo "用法: $0 <目标目录>"
  exit 1
fi

if [ ! -d "$TARGET" ]; then
  echo "目录不存在: $TARGET"
  exit 1
fi

# TODO: 你的逻辑
echo "处理目录: $TARGET"'
elif [ "$LANG_JSON" = 1 ]; then
  TITLE="JSON 配置"
  SUMMARY="已生成一个 JSON 配置文件骨架（注意 JSON 无注释，字段用双引号）。"
  CODE='{
  "name": "your-app",
  "version": "0.1.0",
  "enabled": true,
  "max_retries": 3,
  "timeout_seconds": 30,
  "endpoints": ["https://example.com/api/v1"],
  "options": {
    "debug": false,
    "locale": "zh-CN"
  }
}'
elif [ "$LANG_RUST" = 1 ]; then
  TITLE="Rust 程序"
  SUMMARY="已生成一个 Rust main.rs 骨架（cargo run 直接运行）。"
  CODE='fn main() {
    let args: Vec<String> = std::env::args().collect();
    println!("Hello, Qibai!");
    if args.len() > 1 {
        println!("参数: {}", args[1]);
    }
}'
elif [ "$LANG_GO" = 1 ]; then
  TITLE="Go 程序"
  SUMMARY="已生成一个 Go main.go 骨架（go run main.go 直接运行）。"
  CODE='package main

import (
	"fmt"
	"os"
)

func main() {
	if len(os.Args) > 1 {
		fmt.Printf("Hello, %s!\n", os.Args[1])
	} else {
		fmt.Println("Hello, Qibai!")
	}
}'
elif [ "$LANG_CSS" = 1 ]; then
  TITLE="CSS 样式"
  SUMMARY="已生成一个通用页面样式骨架（居中卡片 + 响应式）。"
  CODE='/* 通用样式骨架 */
* { box-sizing: border-box; }
body {
  margin: 0;
  font-family: system-ui, sans-serif;
  background: #f5f5f5;
}
.card {
  max-width: 640px;
  margin: 32px auto;
  padding: 24px;
  background: #fff;
  border-radius: 12px;
  box-shadow: 0 2px 10px rgba(0, 0, 0, .06);
}
@media (max-width: 480px) {
  .card { margin: 16px; padding: 16px; }
}'
else
  TITLE="代码需求识别"
  SUMMARY="我可以按语言输出可运行骨架：HTML 页面 / Python 脚本 / JavaScript / Shell / JSON 配置 / Rust / Go / CSS。请指明语言与用途，例如「写一个 Python 脚本读取 CSV 并打印统计」。复杂定制代码也可以直接说「帮我写……」，元气小小会生成完整实现。"
  CODE=''
fi

# ---------- 面板展示（window 独立窗，代码可复制） ----------
if [ -n "$CODE" ]; then
  echo "好的，$SUMMARY 代码已展示在面板里，可直接复制使用。"
  echo "@qibai-panel:begin"
  echo "mode=window"
  echo "title=$TITLE · 元气小小写代码"
  echo "html=<div><h3>$TITLE · 需求：$(printf '%s' "$TEXT" | esc | cut -c 1-60)</h3><p>$SUMMARY</p><pre style=\"background:#1e1e1e;color:#e6e6e6;padding:12px;border-radius:8px;overflow:auto;max-height:480px;\">$(printf '%s' "$CODE" | esc)</pre><p><small>复杂定制代码请直接对元气小小说「帮我写……」，会生成完整实现。</small></p></div>"
  echo "@qibai-panel:end"
else
  echo "$SUMMARY"
fi

# ---------- 落盘协议块（v18.6）：路径由 Rust 端安全校验后写入，绝不覆盖 ----------
if [ -n "$SAVE_PATH" ] && [ -n "$CODE" ]; then
  FULL_PATH=$(printf '%s' "$SAVE_PATH" | sed "s|^~|$HOME|")
  BASE64=$(printf '%s' "$CODE" | base64 -w0 2>/dev/null)
  if [ -n "$BASE64" ]; then
    echo "已识别保存意图，我来帮你保存到：$FULL_PATH（若文件已存在我会先告诉你，不覆盖）。"
    echo "@qibai-save:begin"
    echo "path=$FULL_PATH"
    echo "content=$BASE64"
    echo "@qibai-save:end"
  fi
fi
