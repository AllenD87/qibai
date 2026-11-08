#!/usr/bin/env bash
# ============================================================
# 兼容入口（已合并）：sherpa-onnx 模型补齐（≈233MB 主模型 + 可选标点）
# ------------------------------------------------------------
# 实现已合并到仓库根 `fetch_resources.sh`（sha256 校验 + 镜像回退 + 开关齐全），
# 本脚本只转发，保留旧路径以兼容 `src-tauri/pipecat/README.md`、`scripts/pipecat_voice.sh`
# 等历史引用，避免两处逻辑各自漂移。
#
# 用法：工程根运行  bash scripts/download_sherpa_models.sh
#       （补齐全部缺失资源；已存在的自动跳过，幂等）
# ============================================================
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
echo "[download_sherpa_models] 该入口已合并，转发到根目录 fetch_resources.sh（含 sha256 校验与镜像回退）" >&2
exec bash "$ROOT/fetch_resources.sh" "$@"
