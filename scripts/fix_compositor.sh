#!/usr/bin/env bash
# =============================================================================
# 元气小小 v45.2 合成器一键诊断/恢复（fix_compositor.sh）
#
# 优先级（用户第二次合成器尝试失败教训）：
#   第1步永远先查 WM 是否存活（早于一切合成器尝试）——上一轮 pkill -x compiz
#   可能已把 WM 杀了而 picom 没起，桌面处于无 WM 状态，窗口装饰/拖动/焦点全废。
#   再判 X 是否具备 COMPOSITE 扩展，最后才谈透明。
#
# 用法：
#   bash scripts/fix_compositor.sh                 # 只读诊断 + 前台试跑 + 分支结论
#   QIBAI_FIX_DRY=1 bash scripts/fix_compositor.sh # dry-run：只打印将执行的命令
#   bash scripts/fix_compositor.sh --restore-wm    # 显式恢复 WM（默认只打印不执行）
#   bash scripts/fix_compositor.sh --replace-running # 显式结束占用的旧合成器（打印 PID 确认）
#
# 安全：默认只读 + 前台试跑（不后台化吞错误）；任何 kill/启动用户桌面动作都要
#       显式开关，绝不无条件 pkill。退出码：0=PASS 1=FAIL 2=BLOCKED。
#       dry-run（QIBAI_FIX_DRY=1）一律 exit 0（只打印将执行的命令，诊断跑完即止）。
# =============================================================================
set -u

OUT="verify-switch-out"
mkdir -p "$OUT"
DIAG="$OUT/fix-compositor/diag.txt"
mkdir -p "$(dirname "$DIAG")"
: > "$DIAG"

DRY=0; [ "${QIBAI_FIX_DRY:-0}" = "1" ] && DRY=1
RESTORE_WM=0; for a in "$@"; do [ "$a" = "--restore-wm" ] && RESTORE_WM=1; done
REPLACE_RUNNING=0; for a in "$@"; do [ "$a" = "--replace-running" ] && REPLACE_RUNNING=1; done

say()  { echo "[$(date +%H:%M:%S)] $*" | tee -a "$DIAG"; }
run()  { # 只读命令：原样跑并落 diag；DRY 只打印
  if [ "$DRY" = 1 ]; then echo "  DRY: $*" | tee -a "$DIAG"; else "$@" 2>&1 | tee -a "$DIAG"; fi
}

# 大字告警
banner() {
  echo ""
  echo "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!" | tee -a "$DIAG"
  echo "!! $*" | tee -a "$DIAG"
  echo "!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!" | tee -a "$DIAG"
  echo ""
}

say "===== 元气小小合成器诊断 fix_compositor (DRY=$DRY RESTORE_WM=$RESTORE_WM) ====="

# ---------------------------------------------------------------------------
# 0. 环境 / DISPLAY 一致性（最常见的「怎么都起不来」根因）
# ---------------------------------------------------------------------------
say "--- [1] DISPLAY / 环境一致性 ---"
echo "DISPLAY=[${DISPLAY:-}] XAUTHORITY=[${XAUTHORITY:-}] WAYLAND_DISPLAY=[${WAYLAND_DISPLAY:-}]" | tee -a "$DIAG"
echo "--- /tmp/.X11-unix ---" | tee -a "$DIAG"; run ls /tmp/.X11-unix/
echo "--- loginctl sessions ---" | tee -a "$DIAG"; run loginctl list-sessions --no-legend
echo "--- 桌面进程真实 DISPLAY/XAUTHORITY ---" | tee -a "$DIAG"
for p in $(pgrep -x gnome-shell -x plasmashell -x compiz -x kwin_x11 -x openbox -x i3 2>/dev/null); do
  echo "pid=$p:" | tee -a "$DIAG"
  tr '\0' '\n' < /proc/$p/environ 2>/dev/null | grep -E '^(DISPLAY|XAUTHORITY)=' | tee -a "$DIAG"
done

# ---------------------------------------------------------------------------
# 1. WM 是否存活（最高优先级，早于一切合成器动作）
# ---------------------------------------------------------------------------
say "--- [2] WM 存活检测 (_NET_SUPPORTING_WM_CHECK) ---"
WM_ID="$(xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null | awk '{print $NF}')"
WM_NAME=""
if [ -n "$WM_ID" ] && [ "$WM_ID" != "0x0" ]; then
  WM_NAME="$(xprop -id "$WM_ID" _NET_WM_NAME 2>/dev/null)"
fi
echo "_NET_SUPPORTING_WM_CHECK=[$WM_ID]  WM_NAME=[$WM_NAME]" | tee -a "$DIAG"

WM_ALIVE=1
# WM id 合法必须是 0x 开头的十六进制；"not found"/空/0x0 都算无 WM。
if ! echo "$WM_ID" | grep -qE '^0x[0-9a-fA-F]+$' || echo "$WM_NAME" | grep -q "not found"; then
  WM_ALIVE=0
  banner "当前没有窗口管理器（上次 pkill -x compiz 可能已把它杀了）！"
  echo "窗口装饰/拖动/焦点都会异常——先恢复桌面再谈透明。" | tee -a "$DIAG"
  # 按桌面给恢复命令（默认只打印）
  if command -v compiz >/dev/null 2>&1; then
    echo "  检测到 compiz：恢复命令 → compiz --replace &" | tee -a "$DIAG"
  fi
  if pgrep -x gnome-shell >/dev/null 2>&1; then
    echo "  检测到 GNOME：Alt+F2 输入 r 回车，或 systemctl --user restart gnome-shell" | tee -a "$DIAG"
  fi
  if pgrep -x kwin_x11 >/dev/null 2>&1 || pgrep -x plasmashell >/dev/null 2>&1; then
    echo "  检测到 KDE：恢复命令 → kwin_x11 --replace &" | tee -a "$DIAG"
  fi
  if [ "$RESTORE_WM" = 1 ]; then
    say "--restore-wm：执行 WM 恢复..."
    if command -v compiz >/dev/null 2>&1; then ( compiz --replace & ) 2>/dev/null;
    elif pgrep -x kwin_x11 >/dev/null 2>&1; then ( kwin_x11 --replace & ) 2>/dev/null; fi
    sleep 2
    WM_ID_NOW="$(xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null | awk '{print $NF}')"
    say "恢复后 _NET_SUPPORTING_WM_CHECK=[$WM_ID_NOW]"
  else
    say "默认只打印不执行；需要真恢复请加 --restore-wm"
  fi
fi

# ---------------------------------------------------------------------------
# 2. X 是否支持 COMPOSITE 扩展（无则立刻判死，停后续）
# ---------------------------------------------------------------------------
say "--- [3] X COMPOSITE 扩展 ---"
COMPOSITE_OK=1
if ! command -v xdpyinfo >/dev/null 2>&1; then
  say "xdpyinfo 缺失，无法判断 COMPOSITE 扩展（按未知处理，继续试跑）"
else
  XCOMP="$(xdpyinfo 2>/dev/null | grep -i composite || echo 'NOT_FOUND')"
  echo "xdpyinfo composite: $XCOMP" | tee -a "$DIAG"
  echo "$XCOMP" | grep -qi composite || COMPOSITE_OK=0
fi

# ---------------------------------------------------------------------------
# 3. 现有合成器进程 / 版本
# ---------------------------------------------------------------------------
say "--- [4] 合成器版本与占用进程 ---"
run command -v picom; run picom --version
run command -v xcompmgr
run command -v compiz
say "--- 正在跑的合成器 ---"
run pgrep -a -x picom
run pgrep -a -x compiz
run pgrep -a -x xcompmgr
say "--- picom 日志尾部 ---"
tail -30 ~/.cache/picom/picom.log 2>/dev/null | tee -a "$DIAG" || echo "(无 picom.log)" | tee -a "$DIAG"

# ---------------------------------------------------------------------------
# 分支判定
# ---------------------------------------------------------------------------
# a) 无 COMPOSITE
if [ "$COMPOSITE_OK" = 0 ]; then
  echo ""
  echo "BLOCKED 本会话 X 不支持 COMPOSITE 扩展" | tee -a "$DIAG"
  echo "  替代方案：远程桌面走 Wayland+portals；本机用原生 X 会话；Xvfb 里谈透明无意义。" | tee -a "$DIAG"
  exit 2
fi

# b) 已有合成器占用
OCCUPANT="$(pgrep -a -x picom 2>/dev/null; pgrep -a -x compiz 2>/dev/null; pgrep -a -x xcompmgr 2>/dev/null)"
if [ -n "$OCCUPANT" ]; then
  echo ""
  echo "INFO 已有合成器在跑：" | tee -a "$DIAG"
  echo "$OCCUPANT" | tee -a "$DIAG"
fi

# ---------------------------------------------------------------------------
# 4. 前台试跑 picom（禁止后台化吞输出）
# ---------------------------------------------------------------------------
say "--- [5] 前台试跑 picom（拿真错误，exit 124=其实起来了在跑）---"
try_backend() {
  local be="$1"; shift
  say "前台 timeout 8s：picom --backend $be --diagnostics"
  if [ "$DRY" = 1 ]; then echo "  DRY: timeout 8 picom --backend $be --diagnostics" | tee -a "$DIAG"; return 0; fi
  timeout 8 picom --backend "$be" --diagnostics 2>&1 | tee -a "$DIAG" | tail -40
  local ec=${PIPESTATUS[0]}
  say "  exit=$ec (124=超时=它起来了在跑)"
  return $ec
}

# c) 先 xrender（最稳），失败再 glx，再失败 xcompmgr
PICOM_UP=0
if command -v picom >/dev/null 2>&1; then
  try_backend xrender; [ $? -eq 124 ] && PICOM_UP=1
  if [ "$PICOM_UP" = 0 ]; then
    try_backend glx; [ $? -eq 124 ] && PICOM_UP=1
  fi
fi
if [ "$PICOM_UP" = 0 ] && command -v xcompmgr >/dev/null 2>&1; then
  say "降级 xcompmgr -c -f 前台试跑"
  if [ "$DRY" = 1 ]; then echo "  DRY: timeout 8 xcompmgr -c -f" | tee -a "$DIAG"
  else timeout 8 xcompmgr -c -f 2>&1 | tee -a "$DIAG" | tail -20; [ ${PIPESTATUS[0]} -eq 124 ] && PICOM_UP=1; fi
fi

# d) 前台跑完回读
say "--- [6] 回读 picom 进程 ---"
RUNNING="$(pgrep -a -x picom 2>/dev/null)"
echo "running picom: [$RUNNING]" | tee -a "$DIAG"
if [ -z "$RUNNING" ] && [ "$PICOM_UP" = 0 ]; then
  echo "WARN picom 前台跑完 8s 后进程为空=已退出，死因见上方 diag.txt 尾部" | tee -a "$DIAG"
fi

# ---------------------------------------------------------------------------
# 5. 自证：_NET_WM_CM_S0 selection
# ---------------------------------------------------------------------------
if [ "$DRY" = 1 ]; then
  say "DRY 结束（所有命令仅打印未执行），dry-run 规则退出码=0"
  exit 0
fi
say "--- [7] 自证 _NET_WM_CM_S0 ---"
CM="$(xprop -root _NET_WM_CM_S0 2>/dev/null)"
echo "raw: $CM" | tee -a "$DIAG"

if [ "$WM_ALIVE" = 0 ]; then
  echo "BLOCKED 无窗口管理器（先恢复 WM 再谈合成器透明）" | tee -a "$DIAG"
  exit 2
fi
if echo "$CM" | grep -qE "window id # 0x[0-9a-fA-F]+"; then
  SEL=$(echo "$CM" | grep -oE "0x[0-9a-fA-F]+")
  echo "PASS 合成器已启用 selection=$SEL" | tee -a "$DIAG"
  exit 0
else
  echo "FAIL 合成器未启用（_NET_WM_CM_S0 仍 [$CM]）" | tee -a "$DIAG"
  echo "  已尝试：xrender/glz 前台、xcompmgr 降级；详见 $DIAG" | tee -a "$DIAG"
  [ "$OCCUPANT" = "" ] || echo "  若报 'Another composite manager already running'，加 --replace-running 显式结束旧进程后重试" | tee -a "$DIAG"
  exit 1
fi
