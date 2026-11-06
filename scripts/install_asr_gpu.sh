#!/usr/bin/env bash
# ============================================================
# 元气小小 · GPU ASR（SenseVoiceSmall/FunASR）真机安装脚本
# ------------------------------------------------------------
# 幂等：可重复执行；已装则跳过。
# 缓存复用：真机 HF 缓存已有 models--FunAudioLLM--SenseVoiceSmall，
#           脚本优先复用，不重新下载。
# 环境约定：
#   HF_ENDPOINT=https://hf-mirror.com   （仅缓存缺失时才走镜像）
#   PIECAT_PYTHON=/path/to/python       （conda 备选环境切换，可选）
#   PIP 源：统一阿里云镜像（-i https://mirrors.aliyun.com/pypi/simple/
#           --trusted-host mirrors.aliyun.com）；若已配置全局 pip 源可忽略。
# 装好后：ASR_BACKEND 缺省 auto → GPU 自动选 sensevoice（config.py 已有逻辑）。
# ============================================================
set -euo pipefail

PY="${PIECAT_PYTHON:-python3}"
PIP_ALIYUN=(-i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com)
echo "[asr-gpu] using python: $PY ($($PY --version 2>&1))"

# 1. 复用真机 HF 缓存（存在则不重下）
HF_CANDIDATES=(
  "$HOME/.cache/huggingface/hub/models--FunAudioLLM--SenseVoiceSmall"
  "$HOME/.cache/modelscope/hub/models--FunAudioLLM--SenseVoiceSmall"
)
CACHE_HIT=0
for c in "${HF_CANDIDATES[@]}"; do
  if [ -d "$c" ]; then
    echo "[asr-gpu] 命中现有模型缓存: $CACHE_HIT"
    CACHE_HIT=1
  fi
done
export HF_ENDPOINT="${HF_ENDPOINT:-https://hf-mirror.com}"
[ "$CACHE_HIT" = "1" ] && echo "[asr-gpu] 复用缓存，不触发下载" || echo "[asr-gpu] 缓存未命中，将经 $HF_ENDPOINT 拉取（≈936MB）"

# 2. 依赖（幂等：已满足则 pip 跳过）
echo "[asr-gpu] pip install funasr modelscope torchaudio （阿里云镜像）..."
"$PY" -m pip install --quiet "${PIP_ALIYUN[@]}" funasr modelscope torchaudio || {
  echo "[asr-gpu] 主环境装依赖失败。py3.9+torch 冲突时，用 conda 备选环境："
  echo "  conda create -n qibai-voice python=3.10 -y && conda activate qibai-voice"
  echo "  conda install pytorch torchaudio pytorch-cuda=12.1 -c pytorch -c nvidia -y"
  echo "  pip install -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com funasr modelscope edge-tts websockets httpx numpy"
  echo "  然后：PIECAT_PYTHON=$(which python3) bash scripts/pipecat_voice.sh"
  exit 1
}

# 3. 冒烟：加载模型（复用缓存）
echo "[asr-gpu] 冒烟：import funasr + 加载 SenseVoiceSmall ..."
"$PY" - <<'PYEOF'
import os
os.environ.setdefault("HF_ENDPOINT", "https://hf-mirror.com")
import funasr
m = funasr.AutoModel(model="FunAudioLLM/SenseVoiceSmall", device="cuda", disable_update=True)
print("[asr-gpu] SenseVoiceSmall loaded on cuda OK")
PYEOF

echo "[asr-gpu] 完成。启动服务：ASR_BACKEND=auto bash scripts/pipecat_voice.sh"
echo "          （auto 检测 CUDA → asr=sensevoice；打印锚点 [server] asr=sensevoice）"
