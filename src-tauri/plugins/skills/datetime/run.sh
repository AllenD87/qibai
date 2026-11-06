#!/bin/sh
# 日期时间技能：本地系统时间。无网络、无文件副作用。
set -u

NOW=$(date '+%Y年%m月%d日 %H:%M')
WEEK_EN=$(date '+%A')
case "$WEEK_EN" in
  Monday)    WEEK='星期一' ;;
  Tuesday)   WEEK='星期二' ;;
  Wednesday) WEEK='星期三' ;;
  Thursday)  WEEK='星期四' ;;
  Friday)    WEEK='星期五' ;;
  Saturday)  WEEK='星期六' ;;
  Sunday)    WEEK='星期日' ;;
  *)         WEEK="$WEEK_EN" ;;
esac

echo "现在是 $NOW，$WEEK"
