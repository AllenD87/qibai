#!/bin/sh
# 计算器技能：本地安全求值。
# 安全设计：字符白名单（仅数字/空格/+-*/%().），杜绝任意代码注入；无网络、无文件副作用。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"
EXPR=$(printf '%s' "$ARGS" | sed -n 's/.*"expression"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$EXPR" ]; then
  EXPR=$(printf '%s' "$ARGS" | sed -e 's/^{"expression"://' -e 's/}$//' -e 's/^"//' -e 's/"$//' | tr -d '{}[]"')
fi
[ -z "$EXPR" ] && { echo '请告诉我算式，比如：使用计算器，算一下 12*8+3。'; exit 0; }

# 白名单过滤：只保留数字、空格与 + - * / % ( ) .
SAFE=$(printf '%s' "$EXPR" | tr -cd '0-9+\-*/%(). ')
if [ "$SAFE" != "$EXPR" ]; then
  echo '算式里有我不认识的字符，只支持数字和加减乘除哦'
  exit 0
fi

RESULT=$(python3 -c "print(eval('''${SAFE}'''))" 2>/dev/null)
if [ -z "$RESULT" ]; then
  echo '这个算式我算不了，检查一下格式？'
  exit 0
fi

# 结果太长截断展示
printf '%s = %s\n' "$EXPR" "$(printf '%s' "$RESULT" | cut -c1-60)"
