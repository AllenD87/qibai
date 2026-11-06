#!/usr/bin/env bash
# =============================================================================
# vllm_serve.sh — 元气小小（元气小小）本地 LLM vLLM 双卡部署脚本（幂等）
# 真机：2× NVIDIA RTX A6000 (48GB×2=96GB, Ampere sm_86, BF16)
# 端点：http://127.0.0.1:8000/v1  → 对接 Pipecat：
#   export QIBAI_LLM_BASE_URL=http://127.0.0.1:8000/v1
#   export QIBAI_LLM_MODEL=qwen3-local
# 用法：./vllm_serve.sh {start|stop|status|restart}
# 可覆盖变量：MODEL PORT HOST GPU_UTIL MAX_MODEL_LEN SERVED_NAME VENV_DIR
# 说明：本脚本仅在真机(GPU)执行；沙盒无 GPU 仅做 bash -n 语法检查。
# =============================================================================
set -euo pipefail

# ---------- 可调参数（环境变量覆盖） ----------
MODEL="${MODEL:-Qwen/Qwen3-32B-Instruct-AWQ}"   # 边界档改 Qwen/Qwen3-72B-Instruct-AWQ 并把 GPU_UTIL=0.92 / MAX_MODEL_LEN=32768
SERVED_NAME="${SERVED_NAME:-qwen3-local}"       # Pipecat QIBAI_LLM_MODEL 填这个
HOST="${HOST:-127.0.0.1}"                      # 仅本机监听，隐私优先
PORT="${PORT:-8000}"
GPU_UTIL="${GPU_UTIL:-0.90}"
MAX_MODEL_LEN="${MAX_MODEL_LEN:-32768}"
TP_SIZE="${TP_SIZE:-2}"
VENV_DIR="${VENV_DIR:-$HOME/.venvs/vllm-qibai}"
LOG_DIR="${LOG_DIR:-$HOME/.cache/qibai-vllm}"
PID_FILE="$LOG_DIR/vllm.pid"
LOG_FILE="$LOG_DIR/vllm.log"

# 下载走国内镜像（沙盒/内网通用）
export HF_ENDPOINT="${HF_ENDPOINT:-https://hf-mirror.com}"

mkdir -p "$LOG_DIR"

log()  { echo "[$(date '+%F %T')] $*"; }

is_running() {
  [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null
}

# ---------- 安装依赖（幂等） ----------
ensure_venv() {
  if [[ ! -d "$VENV_DIR" ]]; then
    log "创建 venv: $VENV_DIR"
    python3 -m venv "$VENV_DIR"
  fi
  # shellcheck disable=SC1091
  source "$VENV_DIR/bin/activate"
  python -m pip install --quiet --upgrade pip -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com
  if ! python -c "import vllm" 2>/dev/null; then
    log "安装 vllm（pip 阿里云镜像，CUDA 12.x 官方 wheel，Ampere sm_86 可用）..."
    pip install --quiet -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com vllm
  else
    log "vllm 已安装: $(python -c 'import vllm;print(vllm.__version__)')"
  fi
  # 模型下载器
  python -c "import huggingface_hub" 2>/dev/null || pip install --quiet -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com huggingface_hub
}

# ---------- 模型下载（hf-mirror，幂等，断点续传） ----------
download_model() {
  log "下载模型（走 $HF_ENDPOINT）：$MODEL"
  log "参考体积：32B-AWQ ≈ 19GB；72B-AWQ ≈ 40GB"
  # shellcheck disable=SC1091
  source "$VENV_DIR/bin/activate"
  python - <<'PY'
import os, sys
from huggingface_hub import snapshot_download
snapshot_download(repo_id=os.environ["MODEL"])
log_line = "模型就绪：" + os.environ["MODEL"]
print("[" + __import__("datetime").datetime.now().strftime("%F %T") + "] " + log_line, file=sys.stderr)
PY
}

# ---------- 启动 ----------
do_start() {
  if is_running; then
    log "vLLM 已在运行 (pid $(cat "$PID_FILE"))，无需重复启动"
    return 0
  fi
  ensure_venv
  download_model

  log "启动 vLLM: $MODEL  tp=$TP_SIZE  util=$GPU_UTIL  max_len=$MAX_MODEL_LEN"
  # shellcheck disable=SC1091
  source "$VENV_DIR/bin/activate"
  nohup python -m vllm.entrypoints.openai.api_server \
    --model "$MODEL" \
    --served-model-name "$SERVED_NAME" \
    --tensor-parallel-size "$TP_SIZE" \
    --gpu-memory-utilization "$GPU_UTIL" \
    --max-model-len "$MAX_MODEL_LEN" \
    --host "$HOST" --port "$PORT" \
    --dtype bfloat16 \
    --trust-remote-code \
    >>"$LOG_FILE" 2>&1 &
  echo $! > "$PID_FILE"
  log "日志: $LOG_FILE"

  # 健康检查：轮询 /v1/models（首启含权重加载，最多等 600s）
  log "等待模型加载并健康检查 http://$HOST:$PORT/v1/models ..."
  for i in $(seq 1 120); do
    if curl -sf "http://$HOST:$PORT/v1/models" >/dev/null 2>&1; then
      log "健康检查通过 ✅  端点: http://$HOST:$PORT/v1  模型名: $SERVED_NAME"
      log "Pipecat 切流: export QIBAI_LLM_BASE_URL=http://$HOST:$PORT/v1 QIBAI_LLM_MODEL=$SERVED_NAME"
      return 0
    fi
    if ! is_running; then
      log "进程异常退出，详见 $LOG_FILE" >&2
      return 1
    fi
    sleep 5
  done
  log "超时未就绪，请检查 $LOG_FILE" >&2
  return 1
}

# ---------- 停止 ----------
do_stop() {
  if is_running; then
    local pid; pid="$(cat "$PID_FILE")"
    log "停止 vLLM (pid $pid)"
    kill "$pid" 2>/dev/null || true
    for _ in $(seq 1 30); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 1
    done
    kill -9 "$pid" 2>/dev/null || true
  else
    log "未在运行"
  fi
  rm -f "$PID_FILE"
}

do_status() {
  if is_running; then
    log "运行中 (pid $(cat "$PID_FILE"))"
    curl -sf "http://$HOST:$PORT/v1/models" || log "（进程在但健康检查未通过）"
  else
    log "未运行"
    return 1
  fi
}

case "${1:-start}" in
  start)   do_start ;;
  stop)    do_stop ;;
  status)  do_status ;;
  restart) do_stop; do_start ;;
  *) echo "用法: $0 {start|stop|status|restart}"; exit 1 ;;
esac
