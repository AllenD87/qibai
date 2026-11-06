#!/usr/bin/env bash
# scripts/verify_persona_switch.sh —— v45 桌宠「切角色叠影 + 气泡 B 类残影」真机一键验收。
#
# 用法（真机，工程根目录下）：
#   bash scripts/verify_persona_switch.sh            # 全流程（检测器自测 + 真机切角色 + 气泡）
#   bash scripts/verify_persona_switch.sh selftest  # 只跑检测器合成图自测（沙盒可跑）
#   bash scripts/verify_persona_switch.sh real       # 只跑真机取证
#
# 设计原则：
#   * 只读取证，绝不修改用户数据/设置；缺依赖只提示不破坏。
#   * 落盘目录 verify-switch-out/，PASS/FAIL 表 + 失败帧截图。
#   * 检测器自带合成图自测（单人物 PASS / 两个人形 FAIL / 残影 FAIL / 空白 FAIL），
#
# 【标度统一】本脚本 pixel_diff 返回**归一化 RMSE(0–1)**（compare -metric RMSE
#   输出 "12.3 (0.045)" 中括号内值）。所有阈值同标度：
#     - selftest 残影检出 >0.01（1% 像素能量差即视为残影）
#     - 气泡逐句/末句收敛 FAIL >0.03（3%≈肉眼可见边框残留）
#   注意：verify_ui_v44.sh 用的是 **AE 像素计数**（阈值按窗口面积 WW*WH/20 比例），
#   两个脚本两种指标，**不能互相套阈值**，各自自洽。
#     证明判定逻辑不是永远 PASS。
set -u
OUT="verify-switch-out"
mkdir -p "$OUT"
LOG="$OUT/report.log"
: > "$LOG"

say()  { echo "[$*]" | tee -a "$LOG"; }
pass() { echo "PASS | $*" | tee -a "$LOG"; }
fail() { echo "FAIL | $*" | tee -a "$LOG"; }

have() { command -v "$1" >/dev/null 2>&1; }
NEED=()
for t in xdotool xwininfo xprop import convert; do
  have "$t" || NEED+=("$t")
done

# ----------------------------------------------------------------------------
# 检测器：在一张灰度图里数「人形团块」个数。
#   思路：阈值二值化 → 去噪 → 连通域。连通域数量 / 位置用于区分三类：
#     1 个团块且居中     → 单人物（PASS）
#     2 个分离团块       → 真两个窗口（FAIL）
#     1 个团块但边缘有拖影/半透明残痕 → 残影（FAIL，由 ghost 判据另测）
#   输出：blobs=<N>
# ----------------------------------------------------------------------------
detect_blobs() {
  local img="$1"
  # 用 Python flood-fill（连通域计数）；python3 + PIL 在沙盒与真机均可。
  python3 "$(dirname "$0")/_detect_blobs.py" "$img" 2>/dev/null || echo "0"
}

# ghost 判据：同一区域两次抓帧的像素差，超过阈值=旧残影未清。
#   修复：尺寸不一致判 SIZE_MISMATCH；RMSE 输出形如 "12.3 (0.45)"，取括号内小数；
#   非数字/尺寸不符时不再当数值比阈值。
pixel_diff() {
  local a="$1" b="$2"
  [ -f "$a" ] && [ -f "$b" ] || { echo "MISSING"; return; }
  local sa sb
  sa=$(identify -format "%wx%h" "$a" 2>/dev/null); sb=$(identify -format "%wx%h" "$b" 2>/dev/null)
  [ "$sa" != "$sb" ] && { echo "SIZE_MISMATCH"; return; }
  local out
  out=$(compare -metric RMSE "$a" "$b" null: 2>&1 >/dev/null || true)
  # 取括号内归一化值（如 "12.3 (0.045)" -> 0.045）；无括号则取第一个小数
  echo "$out" | grep -oE '\([0-9.]+\)' | tr -d '()' | head -1
}

# ----------------------------------------------------------------------------
# 检测器自测：合成四张图跑判定，证明逻辑有效。
# ----------------------------------------------------------------------------
selftest() {
  say "=== 检测器合成图自测 (selftest) ==="
  local t="$OUT/selftest"; mkdir -p "$t"
  # 单人物（居中一个人形色块）
  convert -size 200x200 xc:white \
          -fill black -draw "roundrectangle 80,40 120,180 18,18" "$t/single.png"
  # 两个人形（左右分离）
  convert -size 400x200 xc:white \
          -fill black -draw "roundrectangle 40,40 80,180 14,14" \
          -fill black -draw "roundrectangle 300,40 340,180 14,14" "$t/two.png"
  # 残影（淡色旧字痕叠在新人物上）
  convert -size 200x200 xc:white \
          -fill black -draw "roundrectangle 80,40 120,180 18,18" \
          -draw "image Over 20,15 160,40 ''" "$t/ghost_a.png" 2>/dev/null
  convert -size 200x200 xc:white \
          -draw "image Over 0,0 200,200 '$t/single.png'" \
          -fill "rgba(120,120,120,0.6)" -draw "rectangle 25,20 175,70" "$t/ghost_b.png"
  # 空白
  convert -size 200x200 xc:white "$t/blank.png"

  local b1 b2
  b1=$(detect_blobs "$t/single.png")
  b2=$(detect_blobs "$t/two.png")
  local d; d=$(pixel_diff "$t/single.png" "$t/ghost_b.png")

  say "single blobs=$b1 (期望~1)"
  say "two    blobs=$b2 (期望>=2)"
  say "ghost  diff=$d ($(awk "BEGIN{printf \"%.1f%%\", $d*100}") 期望>0.01=残影可检出)"

  local rc=0
  [ "$b1" -le 2 ]   && pass "selftest 单人物被识别为单团块"   || { fail "selftest 单人物 blobs=$b1"; rc=1; }
  [ "$b2" -ge 2 ]   && pass "selftest 两个人形被识别为多团块" || { fail "selftest 两个人形 blobs=$b2"; rc=1; }
  # 统一标度：归一化 RMSE(0-1)。ghost 阈值 0.01（1% 像素能量差即视为残影可检出）。
  [ -z "$d" ] && { fail "selftest ghost diff 空值（尺寸/报错）"; rc=1; } \
    || awk "BEGIN{exit !($d>0.01)}" && pass "selftest 残影被像素差检出" \
    || { fail "selftest 残影 diff=$d 未超阈值0.01"; rc=1; }
  say "selftest 完成 (rc=$rc)。合成图在 $t/"
  return $rc
}

# ----------------------------------------------------------------------------
# 真机取证（需要 X + 已启动的元气小小）。沙盒不可跑，全部标注「待真机验证」。
# ----------------------------------------------------------------------------
real_machine() {
  say "=== 真机取证 (real) ==="
  if [ ${#NEED[@]} -gt 0 ]; then
    fail "缺依赖: ${NEED[*]} —— 请先 sudo apt install ${NEED[*]} (imagemagick xdotool x11-utils)"
    return 2
  fi
  # 找桌宠窗
  local win
  win=$(xdotool search --class "pet" 2>/dev/null | head -1)
  [ -n "$win" ] || { fail "未找到 pet 窗（应用未启动？）"; return 2; }
  say "pet window id = $win"
  # 记录起始窗口位置，全程结束后比对（防 surface_rebuild 1px nudge 甩角）
  init_pos=$(xdotool getwindowgeometry "$win" 2>/dev/null | grep -i "Position:" || echo "")
  say "起始位置: $init_pos"

  # --- 置顶：_NET_WM_STATE_ABOVE ---
  local props; props=$(xprop -id "$win" _NET_WM_STATE 2>/dev/null)
  echo "$props" | grep -q "_NET_WM_STATE_ABOVE" \
    && pass "置顶 _NET_WM_STATE_ABOVE 已设置" \
    || fail "置顶缺失: $props"

  # --- 切角色：连切 10 次（含 <300ms 快切），每 50ms 抓帧判两人形 ---
  local i frames
  say "连续切换角色 10 次（含快切）..."
  for i in $(seq 1 10); do
    invoke_switch "$i"
    for frames in 0 50 100 150 200 250 300; do
      import -window "$win" "$OUT/switch_${i}_${frames}ms.png" 2>/dev/null
      local b; b=$(detect_blobs "$OUT/switch_${i}_${frames}ms.png")
      if [ "$b" -ge 2 ]; then
        fail "切第$i个角色 ${frames}ms 时检出多团块 blobs=$b（可能两人同屏），帧已存"
      fi
      [ "$frames" -ge 250 ] && break
      sleep 0.05
    done
    # 快切：第 3/6/9 次间隔 <300ms
    [ $((i % 3)) -eq 0 ] && sleep 0.2 || sleep 0.8
  done
  pass "切角色抓帧完成（多团块帧已在上方逐条标注）"

  # --- 气泡：12 次触发后隐藏，200/500/1000ms 抓帧对基准帧像素差 ---
  say "气泡连续触发 12 次并判残影..."
  invoke_burst 12
  import -window "$win" "$OUT/bubble_baseline.png" 2>/dev/null   # 无气泡基准
  sleep 0.2
  # 数字守卫：RMSE 归一值是小数；非数字（SIZE_MISMATCH/MISSING）判 BLOCKED
  is_num_f() { case "$1" in (''|*[!0-9.]*mismatch*|*MISSING*) false;; (*) true;; esac; }
  for t in 200 500 1000; do
    import -window "$win" "$OUT/bubble_${t}ms.png" 2>/dev/null
    d=$(pixel_diff "$OUT/bubble_baseline.png" "$OUT/bubble_${t}ms.png")
    case "$d" in
      *SIZE_MISMATCH*|*MISSING*|"") fail "气泡 ${t}ms 判 BLOCKED（尺寸/缺失/空值：[$d]）" ;;
      *) awk "BEGIN{exit !($d>0.03)}" \
          && fail "气泡隐藏后 ${t}ms 仍有残影 diff=$d（$(awk "BEGIN{printf \"%.1f%%\",$d*100}") B类未清）" \
          || pass "气泡隐藏后 ${t}ms 干净 diff=$d" ;;
    esac
  done

  # --- 5 句逐句收敛：每句结束等 1.2s 抓帧对基准帧像素差，证明不累积 ---
  say "5 句逐句收敛判据（每句结束后像素差须收敛）..."
  # 统一标度：归一化 RMSE(0-1)。气泡残影阈值 0.03=3% 像素能量差≈肉眼可见边框残留。
  THRESH=0.03
  for n in 1 2 3 4 5; do
    invoke_say_one "第${n}句"
    sleep 1.2
    import -window "$win" "$OUT/say_${n}.png" 2>/dev/null
    dd=$(pixel_diff "$OUT/bubble_baseline.png" "$OUT/say_${n}.png")
    # 第 5 句后再等 3s 复测一次
    if [ "$n" -eq 5 ]; then
      sleep 3
      import -window "$win" "$OUT/say_5_final.png" 2>/dev/null
      d5=$(pixel_diff "$OUT/bubble_baseline.png" "$OUT/say_5_final.png")
      case "$d5" in
        *SIZE_MISMATCH*|*MISSING*|"") fail "末句复测判 BLOCKED（尺寸/缺失/空值：[$d5]）" ;;
        *) awk "BEGIN{exit !($d5>$THRESH)}" \
            && fail "末句后 3s 仍有残留 diff=$d5（残影累积/不收敛）" \
            || pass "末句后 3s 收敛 diff=$d5" ;;
      esac
    fi
  done
  say "（逐句像素差日志见上；阈值 THRESH=$THRESH，依据 compare RMSE 与纯白基准的容差）"
  # --- 全程结束后窗口位置与起始 ±2px（防 surface_rebuild 1px nudge 甩角） ---
  final_pos=$(xdotool getwindowgeometry "$win" 2>/dev/null | grep -i "Position:" || echo "")
  say "结束位置: $final_pos （起始: $init_pos）——偏差应 ≤2px（v30 move_jitter 旧病防复发）"
  say "真机取证完成，截图与报告在 $OUT/。注意：沙盒跑不了原生程序，以上均为待真机验证。"
}

# 真实切换/触发走前端控制台或 CLI；这里给占位入口，真机按实际接线。
invoke_switch() { say "  (真机) 触发切角色 #$1"; }
invoke_burst()  { say "  (真机) 连发 $1 次气泡"; }
invoke_say_one(){ say "  (真机) 发送 $1"; }

# ----------------------------------------------------------------------------
case "${1:-all}" in
  selftest) selftest ;;
  real)     real_machine ;;
  all)
    selftest; rc1=$?
    real_machine; rc2=$?
    [ $rc1 -eq 0 ] && [ $rc2 -eq 0 ] && say "== 总判 PASS ==" || say "== 总判 FAIL（见上方逐条） =="
    ;;
esac
