#!/bin/sh
# 文件查找技能（只读）：find 按文件名关键词查找，排除 node_modules/.git。
ARGS="$QIBAI_SKILL_ARGS"
NAME=$(printf '%s' "$ARGS" | grep -oE '[A-Za-z0-9_.-]+\.[A-Za-z0-9]+' | tail -1)
[ -z "$NAME" ] && NAME=$(printf '%s' "$ARGS" | sed -n 's/.*[找查]\+\([一-龥A-Za-z0-9_]\{2,\}\).*/\1/p')
if [ -z "$NAME" ]; then
  echo "请告诉我文件名关键词，例如：使用文件查找，找 settings.json"
  exit 0
fi
HIT=$(find "$HOME" -maxdepth 6 -name "*$NAME*" -not -path "*/node_modules/*" -not -path "*/.git/*" -not -path "*/target/*" 2>/dev/null | head -10)
if [ -z "$HIT" ]; then
  echo "在 $HOME 下未找到包含「$NAME」的文件（只读查找）。"
else
  echo "找到（只读查找，未改动任何文件）："
  echo "$HIT"
fi
