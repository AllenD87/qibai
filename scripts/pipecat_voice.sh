#!/usr/bin/env bash
# ============================================================
# 元气小小 · Pipecat Realtime 语音服务启动脚本（A 线）
# ------------------------------------------------------------
# 服务：ws://127.0.0.1:8765/v1/realtime（OpenAI Realtime 兼容）
# 依赖：pip install -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com -r src-tauri/pipecat/requirements.txt（阿里云镜像）
# 模型：src-tauri/models/sherpa/（paraformer.onnx + tokens.txt + silero_vad.onnx）
#       首次运行前先跑 scripts/download_sherpa_models.sh 补齐（≈233MB）
# 环境变量：
#   QIBAI_REALTIME_PORT  默认 8765
#   QIBAI_LLM_BASE_URL   默认 http://100.92.2.152:8080/v1
#   QIBAI_LLM_MODEL      默认 Qwen3.6-35B-A3B
# ============================================================
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/src-tauri/pipecat"

PORT="${QIBAI_REALTIME_PORT:-8765}"
# LLM 端点/模型名必须显式注入（模型名与 vLLM served-name 一致，含 Qwen/ 前缀）
export QIBAI_LLM_BASE_URL="${QIBAI_LLM_BASE_URL:-http://100.92.2.152:8080/v1}"
export QIBAI_LLM_MODEL="${QIBAI_LLM_MODEL:-Qwen/Qwen3.6-35B-A3B}"
# 模型路径配置（SenseVoice/ChatTTS）：由 install_models.sh 写入工程根 models_env.sh
[ -f "$ROOT/models_env.sh" ] && . "$ROOT/models_env.sh"
echo "[pipecat-voice] starting ws://127.0.0.1:${PORT}/v1/realtime  LLM=$QIBAI_LLM_BASE_URL model=$QIBAI_LLM_MODEL ChatTTS=${QIBAI_CHATTTTS_MODEL:-未设置}"
exec python3 -m pipecat_voice.server
