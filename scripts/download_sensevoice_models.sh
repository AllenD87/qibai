#!/usr/bin/env bash
# ============================================================
# 元气小小 · SenseVoiceSmall (GPU 档 ASR) 模型补齐脚本
# ------------------------------------------------------------
# 体积：SenseVoiceSmall ≈ 936MB（fp32；zh/yue/en/ja/ko）
# 通道：HF 走 hf-mirror.com；ModelScope 备选（FunASR 默认源）。
# 触发：仅当 ASR_BACKEND=auto 且检测到 CUDA，或显式 ASR_BACKEND=sensevoice。
# CPU 档不需要本脚本（sherpa int8 见根目录 fetch_resources.sh）。
# ============================================================
set -uo pipefail
export HF_ENDPOINT="${HF_ENDPOINT:-https://hf-mirror.com}"
echo "[sensevoice] HF_ENDPOINT=$HF_ENDPOINT"
python3 - <<'PY'
import os
os.environ.setdefault("HF_ENDPOINT", "https://hf-mirror.com")
try:
    from modelscope import snapshot_download  # ModelScope 备选通道
    p = snapshot_download("FunAudioLLM/SenseVoiceSmall")
    print("[sensevoice] modelscope ->", p)
except Exception as e:
    print(f"[sensevoice] modelscope unavailable ({e}); fallback hf-mirror")
    from huggingface_hub import snapshot_download
    p = snapshot_download("FunAudioLLM/SenseVoiceSmall")
    print("[sensevoice] hf-mirror ->", p)
PY
echo "[sensevoice] 完成（≈936MB）"
