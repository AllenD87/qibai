#!/bin/sh
# 代码检查技能（只读）：node --check 检查目录内 js 文件语法，输出报告，不修改任何文件。
ARGS="$QIBAI_SKILL_ARGS"
DIR=$(printf '%s' "$ARGS" | grep -oE '/[A-Za-z0-9_.+/~-]+' | head -1)
case "$DIR" in
  ~/*) DIR="$HOME${DIR#\~}" ;;
esac
if [ -z "$DIR" ] || [ ! -d "$DIR" ]; then
  echo "请告诉我要检查的目录路径，例如：调用代码检查，检查 /home/用户/项目目录"
  exit 0
fi
COUNT=0; FAIL=0
for f in $(find "$DIR" -type f \( -name "*.js" -o -name "*.mjs" -o -name "*.cjs" \) 2>/dev/null | head -50); do
  COUNT=$((COUNT+1))
  if ! node --check "$f" 2>/tmp/qibai_check_err; then
    FAIL=$((FAIL+1))
    echo "语法错误: $f"
    sed -n '1,3p' /tmp/qibai_check_err 2>/dev/null
  fi
done
if [ "$COUNT" -eq 0 ]; then
  echo "目录 $DIR 下未找到 .js/.mjs/.cjs 文件（只读扫描，未改动任何文件）。"
elif [ "$FAIL" -eq 0 ]; then
  echo "检查完成：$DIR 下共 $COUNT 个 JS 文件，全部通过语法检查，未发现明显问题（只读检查，未改动任何文件）。"
else
  echo "检查完成：$DIR 下共 $COUNT 个 JS 文件，其中 $FAIL 个存在语法错误（见上方详情，只读检查，未改动任何文件）。"
fi
