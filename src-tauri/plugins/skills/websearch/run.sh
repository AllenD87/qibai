#!/bin/sh
# 网络搜索技能：必应（cn.bing.com）只读搜索（免登录、无需 key）。返回前 5 条标题+链接。
# 安全设计：只读 GET、8s 超时、UA 伪装浏览器、无文件副作用。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"
Q=$(printf '%s' "$ARGS" | sed -n 's/.*"query"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$Q" ]; then
  Q=$(printf '%s' "$ARGS" | sed -e 's/^{"query"://' -e 's/}$//' -e 's/^"//' -e 's/"$//' | tr -d '{}[]"')
fi
[ -z "$Q" ] && { echo '请告诉我要搜索什么，比如：使用网络搜索，搜一下今天的新闻。'; exit 0; }

Q_ENC=$(printf '%s' "$Q" | python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.stdin.read().strip()))')
HTML=$(curl -s --max-time 8 -A "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36" "https://cn.bing.com/search?q=${Q_ENC}")

if [ -z "$HTML" ]; then
  echo '搜索没成功（可能没网络），稍后再试试？'
  exit 0
fi

printf '%s' "$HTML" | python3 -c '
import sys, re, html
h = sys.stdin.read()
items = []
for m in re.finditer(r"<li class=\"b_algo\".*?</li>", h, re.S):
    h2 = re.search(r"<h2[^>]*>(.*?)</h2>", m.group(0), re.S)
    href = re.search(r"href=\"(https?://[^\"]+)\"", m.group(0))
    if not h2 or not href:
        continue
    title = re.sub(r"<[^>]+>", "", h2.group(1)).strip()
    url = href.group(1)
    if title and url and "bing.com" not in url and "microsoft" not in url:
        items.append((html.unescape(title), url))
        if len(items) >= 5:
            break
if not items:
    print("没有搜到相关内容，换个关键词试试？")
else:
    for i, (t, u) in enumerate(items, 1):
        print(f"{i}. {t}\n   {u}")
'
