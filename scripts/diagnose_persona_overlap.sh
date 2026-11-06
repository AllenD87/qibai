#!/usr/bin/env bash
# =============================================================================
# 元气小小 v45.2 「切角色仍短暂重叠」三步只读诊断（diagnose_persona_overlap.sh）
#
# 优先级：先确认跑的是不是新二进制 → 再用窗口计数判双窗（不用截图）→
#         单窗仍重叠时与合成器状态绑定给结论。
# 只读：不 kill、不启动、不写用户配置；只写自身输出目录。
#
# 用法：bash scripts/diagnose_persona_overlap.sh
# =============================================================================
set -u

OUT="verify-switch-out"; mkdir -p "$OUT"
DIR="$OUT/diagnose-overlap"; mkdir -p "$DIR"
R="$DIR/result.txt"; : > "$R"
say() { echo "$*" | tee -a "$R"; }

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
say "===== 切角色重叠三步诊断 (ROOT=$ROOT) ====="

# ---------------------------------------------------------------------------
# 第一步：跑的是不是新二进制
# ---------------------------------------------------------------------------
say ""; say "===== 第一步：二进制新鲜度 ====="
BINS=(
  "$ROOT/src-tauri/target/release/qibai-tauri"
  "$ROOT/src-tauri/target/debug/qibai-tauri"
  "$ROOT/src-tauri/target/release/qibai"
)
for b in "${BINS[@]}"; do
  [ -e "$b" ] && say "候选二进制: $(ls -l --time-style=full-iso "$b" | awk '{print $6,$7,$NF}')"
done
command -v qibai >/dev/null 2>&1 && say "command -v qibai = $(command -v qibai)"

# 正在跑的进程用的哪个二进制（最关键）
RUNNING_BIN=""
for pid in $(pgrep -x qibai-tauri 2>/dev/null) $(pgrep -f "qibai" 2>/dev/null | head -3); do
  exe="$(readlink -f /proc/$pid/exe 2>/dev/null)"
  [ -n "$exe" ] && say "进程 pid=$pid → 二进制 $exe (mtime=$(stat -c '%y' "$exe" 2>/dev/null))" && RUNNING_BIN="$exe"
done
[ -z "$RUNNING_BIN" ] && say "未找到正在运行的 qibai 进程（请先启动桌宠）"

# Rust 侧符号命中（pet.js 是资源不一定 grep 到，以 Rust 字符串为准）
MISSING=()
check_sym() {
  local label="$1" pat="$2"
  if [ -n "$RUNNING_BIN" ] && [ -e "$RUNNING_BIN" ]; then
    local n; n=$(grep -a -c "$pat" "$RUNNING_BIN" 2>/dev/null || echo 0)
    say "符号[$label] 命中次数=$n"
    [ "$n" -eq 0 ] && MISSING+=("$label")
  else
    MISSING+=("$label(无运行二进制可验)")
  fi
}
check_sym "persona-hot-swap" "persona-hot-swap"
check_sym "pet_surface_rebuild" "pet_surface_rebuild"
check_sym "pet_invalidate_rect" "pet_invalidate_rect"
check_sym "drag-enter" "drag: enter pet_start_drag"

# 源码新鲜度
say "源码 mtime:"
stat -c '  %y  %n' "$ROOT/src-tauri/src/main.rs" "$ROOT/src/pet.js" 2>/dev/null

BINARY_FRESH=yes
if [ ${#MISSING[@]} -gt 0 ]; then BINARY_FRESH=no; fi
# 二进制 mtime 早于源码 → 未重编
if [ -n "$RUNNING_BIN" ] && [ -e "$RUNNING_BIN" ] && [ -e "$ROOT/src-tauri/src/main.rs" ]; then
  bm=$(stat -c %Y "$RUNNING_BIN"); sm=$(stat -c %Y "$ROOT/src-tauri/src/main.rs")
  if [ "$bm" -lt "$sm" ]; then BINARY_FRESH=no; MISSING+=("二进制早于源码(未重编)"); fi
fi
say "BINARY_FRESH=$BINARY_FRESH  MISSING_SYMBOLS=[${MISSING[*]}]"

if [ "$BINARY_FRESH" != "yes" ]; then
  say "结论：跑的是旧二进制／未重编，切角色仍走「关旧窗建新窗」，现象是旧的——先重编再测。"
  say "  重编命令：export PATH=\"\$HOME/.cargo/bin:\$PATH\" && cd $ROOT/src-tauri && cargo build --release"
  exit 2
fi

# ---------------------------------------------------------------------------
# 第二步：窗口计数判双窗（不用截图）
# ---------------------------------------------------------------------------
say ""; say "===== 第二步：窗口计数判双窗（采样10s）====="
say "当前已有的桌宠窗："
xdotool search --name "元气小小" 2>/dev/null | while read -r wid; do
  say "  wid=$wid geo=$(xwininfo -id "$wid" 2>/dev/null | awk '/Absolute upper-left/{print $4,$6}')"
done
command -v wmctrl >/dev/null 2>&1 && wmctrl -l 2>/dev/null | tee -a "$R"

say ">> 现在请在 10s 内手动切角色至少 3 次（含一次快速连切）<<"
MAX_WINDOWS=1; TRANSITIONS=0
: > "$DIR/window_count.log"
for i in $(seq 1 200); do
  n=$(xdotool search --name "元气小小" 2>/dev/null | wc -l | tr -d ' ')
  ts=$(date +%H:%M:%S.%N | cut -c1-12)
  echo "$ts count=$n" >> "$DIR/window_count.log"
  [ "$n" -gt "$MAX_WINDOWS" ] && MAX_WINDOWS=$n
  if [ "$n" -ge 2 ]; then TRANSITIONS=$((TRANSITIONS+1)); fi
  sleep 0.05
done
say "MAX_WINDOWS=$MAX_WINDOWS  >=2 连续采样点=$TRANSITIONS"

if [ "$MAX_WINDOWS" -ge 2 ]; then
  say "FAIL 双窗路径仍在（MAX=$MAX_WINDOWS）——apply_persona_switch 新分支没走到，或别处仍建窗。"
  say "  排查：Rust 侧 grep create_pet_window 的所有调用点，确认切角色只 emit hot-swap 不建窗。"
  exit 1
fi

# ---------------------------------------------------------------------------
# 第三步：单窗仍重叠 → 与合成器状态绑定
# ---------------------------------------------------------------------------
say ""; say "===== 第三步：单窗 + 合成器状态 ====="
CM="$(xprop -root _NET_WM_CM_S0 2>/dev/null)"
WM="$(xprop -root _NET_SUPPORTING_WM_CHECK 2>/dev/null)"
say "_NET_WM_CM_S0: $CM"
say "_NET_SUPPORTING_WM_CHECK: $WM"
if echo "$CM" | grep -qE "window id # 0x[0-9a-fA-F]+"; then
  say "COMPOSITOR=present"
  say "结论：单窗 + 有合成器 仍重叠 → 真 bug，进代码排查三项："
  say "  a) 切角色时是否先 clearRect 旧立绘矩形再画新图；"
  say "  b) 是否有 CSS cross-fade/opacity 过渡让新旧两图叠加（tauri-mode 禁 transition）；"
  say "  c) persona-hot-swap 是否只换 src 没重算 canvas 尺寸/位置（不同角色宽高比错位叠加）。"
  exit 1
else
  say "COMPOSITOR=missing"
  say "结论：单窗 + 无合成器 → 透明模式 repaintFrame 跳过底色 fill，新立绘只覆盖自身矩形，"
  say "      旧立绘占的那块没被清，两个人像同在，直到下一帧全量重绘/定时器触发才盖掉。"
  say "      这是物理必然不是代码 bug；先跑 scripts/fix_compositor.sh 起合成器，再复测。"
  exit 0
fi
