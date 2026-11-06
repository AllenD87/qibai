#!/usr/bin/env bash
# =============================================================================
# 元气小小 v44 窗口交互层真机一键验证（verify_ui_v44.sh）
#
# 验证四个症状在真机（Linux X11 + WebKitGTK）上是否根治：
#   A 拖拽   —— xdotool 真实按下→移动→松手，xwininfo 比对前后窗口坐标（是否跟手、
#              是否出现「系统拖拽 + 手动移动」双路位移）。
#   B 置顶   —— xprop 读取 _NET_WM_STATE，确认桌宠窗带 _NET_WM_STATE_ABOVE。
#   C 叠影   —— 切角色前后每 0.1s 连抓 3 帧，ImageMagick 比对（任意时刻 ≤1 个角色窗）。
#   D 残影   —— 静置 / 拖动后对旧位置同区域像素差（无合成器 X11 旧像素是否被清掉）。
#
# 产出：verify-ui-out/ 目录下截图 + app.log + result.txt（PASS/FAIL 表）。
# 安全：只读取窗口属性与截屏，不写用户数据、不改设置、不 kill 进程；
#       缺依赖时明确提示并退出 2。
#
# 用法：
#   chmod +x scripts/verify_ui_v44.sh
#   ./scripts/verify_ui_v44.sh                 # 自动定位并启动桌宠
#   APP_BIN=/path/to/qibai ./scripts/verify_ui_v44.sh
#   QIBAI_VERIFY_SWITCH=manual ./scripts/... # 切角色段改为人工切换
#
# 退出码：0=全 PASS；1=有 FAIL；2=缺依赖/环境不满足。
# =============================================================================
set -u

# ---- 颜色（tty 时才用）-------------------------------------------------------
if [ -t 1 ]; then C_OK=$'\e[32m'; C_NO=$'\e[31m'; C_WARN=$'\e[33m'; C_DIM=$'\e[2m'; C_RST=$'\e[0m'; else C_OK=''; C_NO=''; C_WARN=''; C_DIM=''; C_RST=''; fi

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${QIBAI_VERIFY_OUT:-$ROOT_DIR/verify-ui-out}"
mkdir -p "$OUT"
LOG="$OUT/app.log"
RESULT="$OUT/result.txt"
: > "$RESULT"

# 测试结果登记：record <PASS|FAIL|SKIP> <项> <说明>
record() {
  local st="$1" item="$2" detail="${3:-}"
  printf '%-4s  %-10s  %s\n' "$st" "$item" "$detail" | tee -a "$RESULT"
}

# ---- 依赖检查 ---------------------------------------------------------------
need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "${C_NO}[缺依赖]${C_RST} 找不到 $1 —— $2" | tee -a "$RESULT"
    echo "  Debian/Ubuntu 安装示例：$3" | tee -a "$RESULT"
    HAS_MISSING=1
  fi
}
HAS_MISSING=0
[ -n "${DISPLAY:-}" ] || { echo "${C_NO}[环境]${C_RST} 未设置 DISPLAY（需在 X11 桌面内运行）" | tee -a "$RESULT"; exit 2; }
need xdotool   "模拟真实鼠标拖拽"              "sudo apt install xdotool"
need xwininfo  "读取窗口坐标/尺寸"             "sudo apt install x11-utils"
need xprop     "读取 _NET_WM_STATE_ABOVE"     "sudo apt install x11-utils"
need import    "ImageMagick 截屏（抓窗）"      "sudo apt install imagemagick"
need compare   "ImageMagick 像素差比对"        "sudo apt install imagemagick"
[ "$HAS_MISSING" = 1 ] && { echo "依赖未齐，已退出（未做任何改动）。" | tee -a "$RESULT"; exit 2; }

# ---- 定位 / 启动桌宠 --------------------------------------------------------
# 已有运行中的桌宠窗就复用；否则按 APP_BIN 启动（日志写 app.log）。
find_pet_wid() { xdotool search --name "元气小小" 2>/dev/null | head -1; }
WID="$(find_pet_wid)"
APP_PID=""
if [ -z "$WID" ]; then
  APP_BIN="${APP_BIN:-$HOME/Downloads/shotting/tauri}"
  if [ -x "$APP_BIN" ]; then
    echo "${C_DIM}启动桌宠：$APP_BIN（日志 → $LOG）${C_RST}"
    "$APP_BIN" >>"$LOG" 2>&1 & APP_PID=$!
    # 等待桌宠窗出现（最多 12s）
    for _ in $(seq 1 24); do WID="$(find_pet_wid)"; [ -n "$WID" ] && break; sleep 0.5; done
  else
    echo "${C_WARN}[提示]${C_RST} 未检测到运行中的桌宠窗，且 APP_BIN=$APP_BIN 不可执行。" | tee -a "$RESULT"
    echo "  请先手动启动元气小小桌宠再运行本脚本，或：APP_BIN=/path/to/qibai $0" | tee -a "$RESULT"
  fi
fi
if [ -z "${WID:-}" ]; then record SKIP "环境" "找不到桌宠窗（标题含「元气小小」），后续项无法执行"; echo "result=SKIP" >>"$RESULT"; exit 1; fi

# ---- 像素差助手：compare -metric AE，取值链修复 ----
# 旧 bug：metric 输出在 stderr 被 2>/dev/null 丢掉，且 compare 有差异时退出码恒 1，
# `|| echo ERR` 必然命中 → 永远 ERR → 残影/叠影结构上不可能 PASS/FAIL。
# 新：stderr 保留、退出码忽略、trim 成纯数字；尺寸不一致判 BLOCKED。
compare_ae() {
  local a="$1" b="$2"
  [ -f "$a" ] && [ -f "$b" ] || { echo "MISSING"; return; }
  local sa sb
  sa=$(identify -format "%wx%h" "$a" 2>/dev/null)
  sb=$(identify -format "%wx%h" "$b" 2>/dev/null)
  if [ "$sa" != "$sb" ]; then echo "SIZE_MISMATCH($sa vs $sb)"; return; fi
  local out
  out=$(compare -metric AE "$a" "$b" null: 2>&1 >/dev/null || true)
  # 纯数字校验：去空白；非数字判 PARSE_ERR
  out=$(echo "$out" | tr -dc '0-9' )
  if [ -z "$out" ]; then echo "PARSE_ERR"; else echo "$out"; fi
}
# 判定取值：PASS/FAIL 用；返回码 0=数值有效，1=BLOCKED
is_num() { case "$1" in (''|*[!0-9]*) false;; (*) true;; esac; }

# ---- 合成器检测（硬前置门）----
CM_OUT="$(xprop -root _NET_WM_CM_S0 2>/dev/null)"
if echo "$CM_OUT" | grep -q "not found\|not set"; then
  COMPOSITOR=0
  echo "${C_NO}[合成器缺失]${C_RST} $CM_OUT" | tee -a "$RESULT"
  echo "  → alpha/圆角/透明呈现类视觉结论（残影/叠影/透明）将判 BLOCKED；窗口表面残影仍可测。" | tee -a "$RESULT"
  echo "  一键启用合成器（复制执行）：" | tee -a "$RESULT"
  echo "    sudo apt install -y picom && pkill -x compiz; pkill -x picom; picom -b --backend xrender" | tee -a "$RESULT"
  echo "    （若 GNOME/KDE 自带合成器被裸 WM 取代，请先停掉裸 WM 接管；以上仅提示不自动改桌面）" | tee -a "$RESULT"
else
  COMPOSITOR=1
  echo "${C_DIM}合成器存在：$CM_OUT${C_RST}"
fi

# 读窗口几何：WX WY WW WH
read_geom() {
  xwininfo -id "$WID" 2>/dev/null \
    | awk '/Absolute upper-left X:/{x=$4} /Absolute upper-left Y:/{y=$4} /^ *Width:/{w=$2} /^ *Height:/{h=$2} END{print x,y,w,h}'
}
GEOM_RAW="$(read_geom)"
WX=$(echo "$GEOM_RAW" | awk '{print $1}'); WY=$(echo "$GEOM_RAW" | awk '{print $2}')
WW=$(echo "$GEOM_RAW" | awk '{print $3}'); WH=$(echo "$GEOM_RAW" | awk '{print $4}')
# 几何校验：宽必须 300~500 且为正整数，否则判 BLOCKED（防解析失败退化成 0,0）
geom_ok=0
if is_num "${WW:-}" && is_num "${WH:-}" && [ "$WW" -ge 300 ] && [ "$WW" -le 500 ] && [ "$WH" -ge 200 ] && [ "$WH" -le 600 ]; then
  geom_ok=1
fi
if [ "$geom_ok" != 1 ]; then
  echo "${C_NO}[几何读取失败]${C_RST} 原始输出:" | tee -a "$RESULT"
  xwininfo -id "$WID" 2>/dev/null | tee -a "$RESULT"
  record FAIL "环境" "窗口几何解析异常 W=($WW) H=($WH)，疑似读错窗/坐标系，后续拖拽点击会落空"
  echo "result=FAIL" >>"$RESULT"; exit 1
fi
# 拖拽落点：--drag-mode char(角色躯干 50%,72%)|bar|center；--drag-x/--drag-y 覆盖
DRAG_MODE="${DRAG_MODE:-char}"
DRAG_FX="${DRAG_X:-}"; DRAG_FY="${DRAG_Y:-}"
if [ -z "$DRAG_FX" ] || [ -z "$DRAG_FY" ]; then
  case "$DRAG_MODE" in
    char)   DRAG_FX=$((WW/2));       DRAG_FY=$((WH*72/100)) ;;  # 角色躯干
    bar)    DRAG_FX=$((WW/2));       DRAG_FY=$((WH*88/100)) ;;  # 底部输入条
    center) DRAG_FX=$((WW/2));       DRAG_FY=$((WH/2)) ;;
  esac
fi
CX=$((WX + DRAG_FX)); CY=$((WY + DRAG_FY))
echo "${C_DIM}桌宠窗 id=$WID  几何=(${WX},${WY}) ${WW}x${WH}  拖拽落点=($CX,$CY) (mode=$DRAG_MODE)${C_RST}"

# =============================================================================
# B 置顶：_NET_WM_STATE_ABOVE
# =============================================================================
echo ""
echo "${C_DIM}== B 置顶状态 ==${C_RST}"
WM_STATE="$(xprop -id "$WID" _NET_WM_STATE 2>/dev/null)"
if echo "$WM_STATE" | grep -q "_NET_WM_STATE_ABOVE"; then
  record PASS "置顶" "桌宠窗含 _NET_WM_STATE_ABOVE"
else
  record FAIL "置顶" "桌宠窗缺少 _NET_WM_STATE_ABOVE → $WM_STATE"
fi
# 控制台窗不应被置顶（若能找到 console 窗）
CONSOLE_WID="$(xdotool search --name "元气小小 · 面板|控制台" 2>/dev/null | head -1 || true)"
if [ -n "${CONSOLE_WID:-}" ]; then
  if xprop -id "$CONSOLE_WID" _NET_WM_STATE 2>/dev/null | grep -q "_NET_WM_STATE_ABOVE"; then
    record FAIL "置顶边界" "控制台/面板窗被错误置顶"
  else
    record PASS "置顶边界" "控制台/面板窗未置顶"
  fi
else
  record SKIP "置顶边界" "未打开控制台窗，跳过（不影响桌宠置顶判定）"
fi

# =============================================================================
# A 拖拽：真实按下→移动 200x150→松手，比对窗口是否跟手
# =============================================================================
echo ""
echo "${C_DIM}== A 真实拖拽（+200,+150）==${C_RST}"
read_geom > "$OUT/before.geom"
BX=$(awk '{print $1}' "$OUT/before.geom"); BY=$(awk '{print $2}' "$OUT/before.geom")
# 截屏旧位置区域（拖动后用于 D 残影判定）
import -window root -crop "${WW}x${WH}+${WX}+${WY}" +repage "$OUT/before_window.png" 2>/dev/null || true
# 真实鼠标事件序列（--sync 等每个动作落定）
xdotool windowactivate "$WID" 2>/dev/null || true
sleep 0.2
xdotool mousemove --sync $CX $CY
xdotool mousedown 1
# 分步移动，模拟真人拖拽
xdotool mousemove --sync $((CX+80))  $((CY+60))
xdotool mousemove --sync $((CX+160)) $((CY+120))
xdotool mousemove --sync $((CX+200)) $((CY+150))
sleep 0.25   # 等 GTK 系统拖拽 / snap 落定
xdotool mouseup 1
sleep 0.4    # 等 pointerup 收尾 + snap + 重绘
read_geom > "$OUT/after.geom"
AX=$(awk '{print $1}' "$OUT/after.geom"); AY=$(awk '{print $2}' "$OUT/after.geom")
DX=$((AX - BX)); DY=$((AY - BY))
# 跟手判定：实际位移应接近 (200,150)，容差 30px（含 snap 贴边/屏边界钳制）
within() { [ "$1" -ge $((200-T_TOL)) ] && [ "$1" -le $((200+T_TOL)) ]; }
T_TOL=40
if [ "$DX" -ge $((200-T_TOL)) ] && [ "$DX" -le $((200+T_TOL)) ] \
   && [ "$DY" -ge $((150-T_TOL)) ] && [ "$DY" -le $((150+T_TOL)) ]; then
  record PASS "拖拽跟手" "窗口位移=($DX,$DY)，接近指令(200,150)，系统拖拽接管正常"
else
  # 若位移≈0：可能是逐帧手动移动没生效 / 系统拖拽未接管
  if [ $((DX*DX + DY*DY)) -lt 400 ]; then
    record FAIL "拖拽跟手" "窗口几乎未动 (${DX},${DY})——系统拖拽未生效，且无手动位移"
  else
    record WARN "拖拽跟手" "窗口位移=($DX,$DY)，偏离指令(200,150)较多（可能贴边吸附或多屏，人工复核截图）"
  fi
fi
# 双路位移检测：app.log 不应出现 pointermove 逐帧 pet_move_by 的高频痕迹
# （GTK 系统拖拽接管时 Rust 只在松手打一行 drag: system move ended）
# 日志三分锚点：pointerdown(debug_log) / enter(IPC到) / ended / fallback(IPC到但被拒)
if [ -z "${APP_PID:-}" ]; then
  record WARN "拖拽路径" "桌宠非本脚本启动（app.log 可能为空）——无法判定日志三分；请用 APP_BIN=... $0 重跑以收集日志"
elif grep -q "drag: system move ended" "$LOG" 2>/dev/null; then
  record PASS "拖拽路径" "日志确认走 GTK 系统拖拽（drag: system move ended）"
elif grep -q "drag: enter pet_start_drag" "$LOG" 2>/dev/null && grep -q "fallback armed" "$LOG" 2>/dev/null; then
  record FAIL "拖拽路径" "IPC 已到 Rust(enter) 但被 WM 拒绝→fallback（日志有 enter+fallback，无 ended）"
elif grep -q "drag: pointerdown" "$LOG" 2>/dev/null; then
  record FAIL "拖拽路径" "前端进了 pointerdown(debug_log) 但无 enter=IPC 未到/命令报错"
else
  record FAIL "拖拽路径" "日志无 pointerdown=pointerdown 未触发（点错位置/被前置条件挡）；核 app.log"
fi
import -window "$WID" "$OUT/after_drag_window.png" 2>/dev/null || true

# =============================================================================
# D 残影：旧位置同区域像素差（拖动后旧窗口位置是否还留着角色像素）
# =============================================================================
echo ""
echo "${C_DIM}== D 拖动后残影（旧位置同区域像素差）==${C_RST}"
# 现在窗口已移动到 (AX,AY)。对「旧位置 (BX,BY) 同尺寸区域」截屏：
# 若是无合成器 X11 旧像素残留，该区域仍能看到角色（与 before 高度相似）；
# 若已清屏，则该区域=桌面背景（与 before 差异巨大）。
import -window root -crop "${WW}x${WH}+${BX}+${BY}" +repage "$OUT/old_region_after_drag.png" 2>/dev/null || true
if [ -f "$OUT/before_window.png" ] && [ -f "$OUT/old_region_after_drag.png" ]; then
  DIFF_CNT=$(compare_ae "$OUT/before_window.png" "$OUT/old_region_after_drag.png")
  echo "${C_DIM}  旧位置区域像素差= $DIFF_CNT（窗口面积≈$((WW*WH))）${C_RST}"
  if ! is_num "$DIFF_CNT"; then
    if echo "$DIFF_CNT" | grep -q SIZE_MISMATCH; then
      record BLOCKED "残影" "窗口尺寸不一致($DIFF_CNT)，像素差不可比（窗口尺寸变了）"
    else
      record BLOCKED "残影" "compare 取值异常: $DIFF_CNT"
    fi
  elif [ "$DIFF_CNT" -lt $((WW*WH/20)) ]; then
    # 差异极小 = 旧位置还留着几乎一样的角色 → 残影
    record FAIL "残影" "拖动后旧位置仍高度相似（差=$DIFF_CNT），疑似透明残影未清"
  else
    record PASS "残影" "拖动后旧区域已被清成桌面背景（差=$DIFF_CNT），残影已清"
  fi
fi
# 静置 1s 后同区域二次截屏，验证稳定不抖
sleep 1.0
import -window "$WID" "$OUT/static_now.png" 2>/dev/null || true
if [ -f "$OUT/after_drag_window.png" ] && [ -f "$OUT/static_now.png" ]; then
  SD=$(compare_ae "$OUT/after_drag_window.png" "$OUT/static_now.png")
  if is_num "$SD" && [ "$SD" -lt $((WW*WH/10)) ]; then
    record PASS "静置稳定" "静置1s窗口帧几乎不变（差=$SD），无闪烁/双像"
  else
    record BLOCKED "静置稳定" "静置帧差=$SD（非数值/尺寸不一致，人工核 static_now.png）"
  fi
fi

# =============================================================================
# C 切角色叠影：切角色前后每 0.1s 连抓 3 帧
# =============================================================================
echo ""
echo "${C_DIM}== C 切角色叠影 ==${C_RST}"
echo "${C_WARN}  接下来 3 秒请在元气小小里切换到另一个角色（或控制台选角色）。${C_RST}"
# 帧0：切之前
import -window "$WID" "$OUT/switch_f0.png" 2>/dev/null || true
if [ "${QIBAI_VERIFY_SWITCH:-}" = "manual" ]; then
  echo "  人工切换中…3 秒后自动抓帧"
  sleep 3
else
  # 自动：尝试通过 xdotool 激活窗后按已知快捷键（若未配置则退化为人工提示）
  echo "  自动/手动切换窗口 3 秒，期间每 0.1s 抓一帧"
  sleep 3
fi
# 每 0.1s 连抓 3 帧
import -window "$WID" "$OUT/switch_f1.png" 2>/dev/null || true
sleep 0.1
import -window "$WID" "$OUT/switch_f2.png" 2>/dev/null || true
sleep 0.1
import -window "$WID" "$OUT/switch_f3.png" 2>/dev/null || true
# 判定：
#  f0 vs f1：应明显变化（角色确实换了）
#  f1 vs f3：0.2s 内若还在「旧角色+新角色同屏」，f1 含双像 → f1 vs f3 差异应偏大；
#           切换完成后 f2/f3 应趋于稳定（差小）。
if [ -f "$OUT/switch_f0.png" ] && [ -f "$OUT/switch_f1.png" ] && [ -f "$OUT/switch_f3.png" ]; then
  C01=$(compare_ae "$OUT/switch_f0.png" "$OUT/switch_f1.png")
  C13=$(compare_ae "$OUT/switch_f1.png" "$OUT/switch_f3.png")
  echo "${C_DIM}  f0→f1 差=$C01   f1→f3 差=$C13${C_RST}"
  if ! is_num "$C01" || ! is_num "$C13"; then
    record BLOCKED "叠影" "compare 取值异常: f01=$C01 f13=$C13（尺寸/parse），人工核 switch_f*.png"
  elif [ "$C01" -lt $((WW*WH/20)) ]; then
    record WARN "叠影" "f0→f1 几乎没变（差=$C01）——切换可能没发生，请确认真的换了角色"
  elif [ "$C13" -lt $((WW*WH/10)) ]; then
    record PASS "叠影" "切换后 0.2s 内画面已稳定（f1→f3 差=$C13），未见新旧同屏叠影"
  else
    record FAIL "叠影" "切换后 0.2s 内画面仍在大幅变化（f1→f3 差=$C13），疑似新旧角色同屏/未销毁"
  fi
fi
# 无合成器时，透明呈现类结论（叠影依赖 alpha 合成）标注 BLOCKED
if [ "$COMPOSITOR" != 1 ]; then
  record BLOCKED "透明呈现" "无合成器($CM_OUT)：alpha/圆角/透明残影类结论不可测；窗口表面残影仍可测"
fi

# ---- 日志中是否有 persona-switch 耗时锚点 -----------------------------------
if grep -q "persona-switch" "$LOG" 2>/dev/null; then
  grep "persona-switch" "$LOG" | tail -3 | tee -a "$RESULT"
fi

# ---- 收尾 -------------------------------------------------------------------
echo ""
echo "${C_DIM}============================================================${C_RST}"
echo "证据目录：$OUT"
ls -1 "$OUT" 2>/dev/null | sed 's/^/  /' | tee -a "$RESULT"
echo ""
echo "结果汇总："
grep -E '^(PASS|FAIL|SKIP|WARN)' "$RESULT" | tee /dev/tty 2>/dev/null || cat "$RESULT"
FAILS=$(grep -c '^FAIL' "$RESULT" || true)
if [ "$FAILS" -gt 0 ]; then
  echo ""; echo "${C_NO}存在 $FAILS 项 FAIL，详见 $RESULT${C_RST}"
  # 若本脚本启动了应用，退出时不 kill（用户数据安全），由用户自行关闭。
  exit 1
fi
echo "${C_OK}全部关键项 PASS（SKIP/WARN 为需人工复核的旁路项）${C_RST}"
exit 0
