#!/bin/sh
# 待办解析技能（只读解析，无副作用）：把自然语言待办/提醒文本解析为结构化 JSON。
# 供主模型配合内置 set_reminder 使用；本脚本本身不设置任何提醒、不写任何文件。
# 依赖：GNU date（Linux 桌面环境自带）。
# 异常输入：空参数/无法解析时输出 ok=false 的 JSON，优雅失败。
set -u

ARGS="${QIBAI_SKILL_ARGS:-}"
RAW=$(printf '%s' "$ARGS" | sed -n 's/.*"text"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
if [ -z "$RAW" ]; then
  # 非 JSON 场景：去掉可能的 JSON 外壳字符后按纯文本处理
  RAW=$(printf '%s' "$ARGS" | tr -d '{}[]"')
fi
TEXT=$(printf '%s' "$RAW" | tr -s ' ' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')

if [ -z "$TEXT" ]; then
  echo '{"ok":false,"error":"请告诉我要提醒的事项，例如：明天下午3点开会提醒我"}'
  exit 0
fi

# ---------- 日期解析 ----------
TODAY=$(date +%Y-%m-%d)
TODAY_WD=$(date +%u)          # 1=周一 ... 7=周日
DATE="$TODAY"

case "$TEXT" in
  *大后天*) DATE=$(date -d "+3 days" +%Y-%m-%d 2>/dev/null || echo "$TODAY") ;;
  *后天*)   DATE=$(date -d "+2 days" +%Y-%m-%d 2>/dev/null || echo "$TODAY") ;;
  *明天*)   DATE=$(date -d "+1 days" +%Y-%m-%d 2>/dev/null || echo "$TODAY") ;;
  *今天*)   DATE="$TODAY" ;;
esac

# 星期词：下周X / 本周X / 星期X / 周X
WD_CHAR=$(printf '%s' "$TEXT" | sed -n 's/.*下周\([一二三四五六日天]\).*/\1/p')
[ -z "$WD_CHAR" ] && WD_CHAR=$(printf '%s' "$TEXT" | sed -n 's/.*本周\([一二三四五六日天]\).*/\1/p')
[ -z "$WD_CHAR" ] && WD_CHAR=$(printf '%s' "$TEXT" | sed -n 's/.*星期\([一二三四五六日天]\).*/\1/p')
[ -z "$WD_CHAR" ] && WD_CHAR=$(printf '%s' "$TEXT" | sed -n 's/.*周\([一二三四五六日天]\).*/\1/p')
if [ -n "$WD_CHAR" ]; then
  WD=0
  case "$WD_CHAR" in
    一|1) WD=1 ;; 二|2) WD=2 ;; 三|3) WD=3 ;; 四|4) WD=4 ;;
    五|5) WD=5 ;; 六|6) WD=6 ;; 日|天|7) WD=7 ;;
  esac
  if [ "$WD" -ge 1 ] 2>/dev/null; then
    case "$TEXT" in
      *下周*) DAYS=$((7 - TODAY_WD + WD)) ;;
      *)      if [ "$WD" -ge "$TODAY_WD" ]; then DAYS=$((WD - TODAY_WD)); else DAYS=$((7 - TODAY_WD + WD)); fi ;;
    esac
    DATE=$(date -d "+$DAYS days" +%Y-%m-%d 2>/dev/null || echo "$TODAY")
  fi
fi

# ---------- 时间解析 ----------
HOUR=""; MIN=""
HM=$(printf '%s' "$TEXT" | grep -oE '[0-9]{1,2}:[0-5][0-9]' | head -1)
if [ -n "$HM" ]; then
  HOUR=$(printf '%s' "$HM" | cut -d: -f1 | sed 's/^0*//')
  MIN=$(printf '%s' "$HM" | cut -d: -f2)
else
  HOUR=$(printf '%s' "$TEXT" | grep -oE '[0-9]{1,2}点' | head -1 | tr -d '点' | sed 's/^0*//')
  if [ -n "$HOUR" ]; then
    case "$TEXT" in
      *点半*) MIN=30 ;;
      *点[0-9]*分*) MIN=$(printf '%s' "$TEXT" | sed -n 's/.*点\([0-9]\{1,2\}\)分.*/\1/p') ;;
      *) MIN=0 ;;
    esac
  fi
fi

if [ -n "$HOUR" ]; then
  [ -z "$MIN" ] && MIN=0
  MIN=$(printf '%02d' "$MIN" 2>/dev/null || echo "$MIN")
  case "$TEXT" in
    *下午*|*晚上*) [ "$HOUR" -lt 12 ] && HOUR=$((HOUR + 12)) ;;
    *凌晨*)        [ "$HOUR" -eq 12 ] && HOUR=0 ;;
    *中午*)        [ "$HOUR" -eq 11 ] && HOUR=12 ;;
  esac
else
  case "$TEXT" in
    *早上*|*清晨*) HOUR=8;  MIN=00 ;;
    *上午*)        HOUR=9;  MIN=00 ;;
    *中午*)        HOUR=12; MIN=00 ;;
    *下午*)        HOUR=14; MIN=00 ;;
    *晚上*|*傍晚*) HOUR=19; MIN=00 ;;
  esac
fi

# ---------- 合成 ISO 时间 ----------
WHEN_ISO=""
if [ -n "$HOUR" ]; then
  WHEN_ISO=$(date -d "$DATE $HOUR:$MIN" +%Y-%m-%dT%H:%M:00 2>/dev/null || echo "")
fi

# ---------- 事项提取（剥离时间/日期词与填充词） ----------
WHAT=$(printf '%s' "$TEXT" | sed \
  -e 's/大后天//g' -e 's/后天//g' -e 's/明天//g' -e 's/今天//g' \
  -e 's/下周[一二三四五六日天]//g' -e 's/本周[一二三四五六日天]//g' \
  -e 's/星期[一二三四五六日天]//g' -e 's/周[一二三四五六日天]//g' \
  -e 's/[0-9]\{1,2\}:[0-5][0-9]//g' \
  -e 's/[0-9]\{1,2\}点[0-9]\{1,2\}分//g' -e 's/[0-9]\{1,2\}点半//g' -e 's/[0-9]\{1,2\}点//g' \
  -e 's/凌晨//g' -e 's/早上//g' -e 's/清晨//g' -e 's/上午//g' -e 's/中午//g' -e 's/下午//g' -e 's/晚上//g' -e 's/傍晚//g' \
  -e 's/提醒我//g' -e 's/提醒一下//g' -e 's/记得//g' -e 's/别忘了//g' -e 's/帮我//g' -e 's/请//g' -e 's/我要//g' \
  | tr -s ' ' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
[ -z "$WHAT" ] && WHAT="（未识别到具体事项，请补充）"

# ---------- JSON 转义并输出 ----------
ESC_TEXT=$(printf '%s' "$TEXT" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')
ESC_WHAT=$(printf '%s' "$WHAT" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')
[ -n "$HOUR" ] && TIME_F="$HOUR:$MIN" || TIME_F=""
echo "{\"ok\":true,\"raw\":\"$ESC_TEXT\",\"date\":\"$DATE\",\"time\":\"$TIME_F\",\"when_iso\":\"$WHEN_ISO\",\"what\":\"$ESC_WHAT\"}"

# v18.2 面板协议块：气泡提醒卡片
WH_E=$(printf '%s' "$WHAT" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g')
echo "@qibai-panel:begin"
echo "mode=bubble"
echo "title=提醒已记录"
echo "html=<div><b>提醒</b>：$WH_E<br/>时间：$DATE $TIME_F</div>"
echo "@qibai-panel:end"
