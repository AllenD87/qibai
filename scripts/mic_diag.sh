#!/usr/bin/env bash
# 元气小小 麦克风自诊断 v1（不依赖元气小小运行，独立定位麦克风链路）
echo "================ 1. 音频服务状态 ================"
if command -v pactl >/dev/null; then
  pactl info 2>/dev/null | grep -E "Server Name|Default Sink|Default Source" || echo "pactl 无输出（PulseAudio 可能没跑）"
else
  echo "无 pactl 命令"
fi
echo ""
echo "================ 2. 系统输入设备列表 ================"
if command -v pactl >/dev/null; then
  pactl list short sources 2>/dev/null | head -12
else
  arecord -l 2>/dev/null | head -12
fi
echo ""
echo "================ 3. 默认输入源音量 ================"
if command -v pactl >/dev/null; then
  pactl get-source-volume @DEFAULT_SOURCE@ 2>/dev/null || echo "无法读取默认源音量"
fi
echo ""
echo "================ 4. ALSA 输入设备 ================"
arecord -l 2>/dev/null | head -12 || echo "arecord 不可用"
echo ""
echo "================ 5. 实测录音 2 秒（请现在对着麦克风说话）================"
arecord -d 2 -f S16_LE -r 44100 -c 1 /tmp/qibai_mic_test.wav 2>&1 | head -4
python3 - <<'PYEOF'
import wave, audioop
try:
    w = wave.open('/tmp/qibai_mic_test.wav')
    n = w.getnframes(); data = w.readframes(n)
    rms = audioop.rms(data, 2)
    sec = n / w.getframerate()
    print(f"录音 {sec:.1f}s  RMS={rms:.0f}")
    if rms > 800:
        print("判定：✅ 采集到明显声音（麦克风链路 OK）")
    elif rms > 200:
        print("判定：⚠️ 采集到微弱声音（音量偏低，检查系统输入增益）")
    else:
        print("判定：❌ 基本静音（设备选错/驱动/独占问题）")
except Exception as e:
    print("分析失败:", e)
PYEOF
echo ""
echo "================ 6. 耳机检测提示 ================"
echo "如果用耳机麦克风：插好后在系统设置里把输入设备切到耳机，再重跑本脚本"
