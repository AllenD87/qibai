#!/usr/bin/env bash
# 元气小小 P0「回声根治工程（信号级 AEC）」真机一键验证脚本（引导式 checklist，非破坏性）
# 用法：bash scripts/verify_aec.sh
# 结果写入：verify_aec_result_<YYYYmmdd_HHMMSS>.log（工程根目录下）
#
# 覆盖：①A/B 开关（QIBAI_AEC=0 旧半双工 vs 默认启用）②播放期麦克风回声消除听感/录测法
#       ③barge-in 打断（残差档 0.008/6 帧）④voice_diag aec_enabled 字段核对
#       ⑤可选 PulseAudio monitor 回采校准法 ⑥已知缺口提示。
#
# 前提：真机 rustc >= 1.91（sonora 0.2.0 要求）；不足先 rustup 升级。
#       构建：cd src-tauri && cargo build --release
set -u
TS=$(date +%Y%m%d_%H%M%S)
LOG="verify_aec_result_${TS}.log"
PASS=0; FAIL=0; SKIP=0
echo "元气小小 P0 信号级 AEC 真机验证 $(date)" | tee "$LOG"
echo "工程根：$(pwd)" | tee -a "$LOG"

# ask <步骤号> <操作> <预期> <判定标准> <备注>
ask() {
  local id="$1" op="$2" exp="$3" judge="$4" note="$5"
  echo "" | tee -a "$LOG"
  echo "【步骤 $id】$op"  | tee -a "$LOG"
  echo "  预期：$exp"      | tee -a "$LOG"
  echo "  判定：$judge"    | tee -a "$LOG"
  echo "  备注：$note"      | tee -a "$LOG"
  printf "  结果 (y=通过 / n=失败 / s=跳过): "
  read -r ans </dev/tty
  case "$ans" in
    y|Y) echo "$id  PASS" | tee -a "$LOG"; PASS=$((PASS+1));;
    n|N) echo "$id  FAIL" | tee -a "$LOG"; FAIL=$((FAIL+1));;
    *)   echo "$id  SKIP" | tee -a "$LOG"; SKIP=$((SKIP+1));;
  esac
}

echo "" | tee -a "$LOG"
echo "════════ 前置：环境与合流核对 ════════" | tee -a "$LOG"

ask P0.0 "rustc --version" \
  "≥ 1.91（sonora 0.2.0 硬性要求）" \
  "版本号 ≥ 1.91；不足则 rustup update 后重编" \
  "本机当前: $(rustc --version 2>/dev/null || echo '未找到 rustc')"

ask P0.1 "在工程根执行 bash scripts/check_qibai_version.sh" \
  "11 锚点全 PASS" \
  "汇总 PASS=11 FAIL=0（v20 P0/SND-1/fix33/fix34/fix29 全在位）" \
  "合流完整性核对"

ask P0.2 "构建：cd src-tauri && cargo build --release" \
  "0 error 编译通过" \
  "target/release/ 下产物生成；stderr 无 error" \
  "首次编译约数分钟"

echo "" | tee -a "$LOG"
echo "════════ ① A/B 开关对照（先关后开，基线对比） ════════" | tee -a "$LOG"
echo "  建议顺序：先 QIBAI_AEC=0 跑一遍旧行为做基线，再默认（启用）对比。" | tee -a "$LOG"

ask A1.1 "旧半双工基线：QIBAI_AEC=0 ./target/release/<桌宠二进制>" \
  "朗读期间麦克风硬静音门生效（旧行为）" \
  "朗读时插话不响应（被硬门丢帧）；打断靠能量门 0.02/8帧" \
  "终端日志应见 [qibai-aec] enabled=false"

ask A1.2 "默认启用：直接 ./target/release/<桌宠二进制>（不设 QIBAI_AEC）" \
  "启动日志 [qibai-aec] enabled=true" \
  "stderr 首屏出现 [qibai-aec] enabled=true" \
  "aec_enabled() 实时读环境变量，运行期可改环境变量重启切换"

echo "" | tee -a "$LOG"
echo "════════ ② 播放期麦克风侧回声消除（听感 + 录测法） ════════" | tee -a "$LOG"
echo "  录测示例：另一终端执行" | tee -a "$LOG"
echo "    arecord -f S16_LE -r 16000 -c 1 -d 10 /tmp/aec_test.wav" | tee -a "$LOG"
echo "  录制期间让桌宠朗读一段长回复（不要说话）；停后：" | tee -a "$LOG"
echo "    aplay /tmp/aec_test.wav   # 回放听残留回声量" | tee -a "$LOG"

ask B2.1 "对照 A：QIBAI_AEC=0 时录 10s 朗读段并回放" \
  "回放能明显听到 TTS 朗读串音（回声大）" \
  "这是旧基线，回声可闻" \
  "基线样本，命名 /tmp/aec_off.wav"

ask B2.2 "对照 B：默认启用时同法录 10s 朗读段并回放" \
  "回放残留朗读声显著变轻/基本不可闻" \
  "与 /tmp/aec_off.wav 对比，回声感明显下降（ERLE 目标 ≥10dB）" \
  "样本命名 /tmp/aec_on.wav"

ask B2.3 "启用期间正常对话一次（不插话）" \
  "回答清晰可懂，未被过度降噪闷住" \
  "近端语音保留（AEC 不应把正常说话消掉）" \
  "对应单测 echo_link_converges 的「近端保留」断言"

echo "" | tee -a "$LOG"
echo "════════ ③ barge-in 打断（残差档 0.008 / 6 帧 ≈120ms） ════════" | tee -a "$LOG"

ask C3.1 "桌宠长回复朗读到一半时，直接说「停」/「闭嘴」" \
  "朗读被立刻打断（≈120ms 内），toast/日志见 barge-in" \
  "stderr 出现 [qibai] barge-in: 检测到用户语音，请求打断朗读" \
  "残差已消回声，0.008 低门即触发"

ask C3.2 "朗读期间故意咳嗽/敲桌放小音量背景声" \
  "不误触发打断，朗读继续" \
  "弱噪声不越过 0.008 能量门 × 连续 6 帧" \
  "灵敏度与抗噪需兼得；误触频繁则记录"

ask C3.3 "打断后 0.4s 尾音窗口内再说话" \
  "尾音结束后监听恢复，可继续对话" \
  "不被自己尾音二次触发，也不静默卡死" \
  "watchdog 90s 兜底：TTS_SPEAKING 残留会被强制解除"

echo "" | tee -a "$LOG"
echo "════════ ④ voice_diag 字段核对 ════════" | tee -a "$LOG"

ask D4.1 "打开诊断面板（或调 voice_diag 命令）" \
  "输出 JSON 含 aec_enabled 布尔字段" \
  "默认运行显示 aec_enabled: true；QIBAI_AEC=0 重启后为 false" \
  "main.rs voice_diag 挂载点"

ask D4.2 "朗读期间观察诊断面板 rms 值（残差 RMS）" \
  "TTS 播放期 rms 不再是过去的「扬声器串音满格」，而是压低后的小值" \
  "播放期 RMS 明显低于关 AEC 时的同场景读数" \
  "残差同时喂 barge-in 与 whisper"

echo "" | tee -a "$LOG"
echo "════════ ⑤ 可选：PulseAudio monitor 回采校准法（延时实测） ════════" | tee -a "$LOG"
echo "  若听感改善不明显，先校准播放→采集延时（引擎 hint=40ms，靠 AEC3 自估）：" | tee -a "$LOG"
echo "    pactl list short sources                 # 找 monitor 源名，通常 <sink>.monitor" | tee -a "$LOG"
echo "    parec -d <sink>.monitor --format=s16le -r 16000 -c 1 /tmp/ref_monitor.raw &" | tee -a "$LOG"
echo "    arecord -f S16_LE -r 16000 -c 1 -d 10 /tmp/cap.raw" | tee -a "$LOG"
echo "  两路 raw 互相关峰值位置即实际延时；若实测延时偏离 30~50ms 较远，" | tee -a "$LOG"
echo "  记录数值并反馈（aec.rs STREAM_DELAY_MS=40 为可调 hint）。" | tee -a "$LOG"

ask E5.1 "（可选）按上法实测播放→采集延时" \
  "实测延时落在 30~50ms 附近，与 STREAM_DELAY_MS=40 hint 相容" \
  "记录实测值；偏离大则在反馈里附数值" \
  "跳过则选 s"

echo "" | tee -a "$LOG"
echo "════════ ⑥ 已知缺口提示（不阻塞，记录即可） ════════" | tee -a "$LOG"

echo "  · espeak/spd-say 直出路径（系统 speech-dispatcher 直通、未经本地 wav 落盘）" | tee -a "$LOG"
echo "    无参考信号 tap —— 该路径播放时回声消除不生效，属已知缺口；" | tee -a "$LOG"
echo "    本机走 bundled-espeak→paplay（已 tap，tts.rs:1257）或 piper/edge-tts 不受影响。" | tee -a "$LOG"
ask F6.1 "确认本机实际 TTS 引擎（日志 [qibai] tts engine: 行）" \
  "记录实际引擎：edge-tts / piper / espeak" \
  "若落 bundled-espeak→paplay 或 piper/edge-tts：AEC 覆盖正常；" \
  "若走 spd-say 直出：记录「已知缺口，该路径回声消除不生效」"

echo "" | tee -a "$LOG"
echo "════ 汇总：PASS=$PASS  FAIL=$FAIL  SKIP=$SKIP ════" | tee -a "$LOG"
echo "完整记录：$LOG" | tee -a "$LOG"
if [ "$FAIL" -gt 0 ]; then
  echo "注意：$FAIL 项未过 —— 优先回看 ②B2.2（ERLE）与 ③C3.1（打断）；" | tee -a "$LOG"
  echo "      仍怀疑合流问题则复跑 bash scripts/check_qibai_version.sh。" | tee -a "$LOG"
fi
exit 0
