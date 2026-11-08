#!/usr/bin/env bash
# ============================================================
# 兼容入口（已合并）：sherpa-onnx 中文 STT（Paraformer-zh）资源补齐
# ------------------------------------------------------------
# 真正的实现已合并到仓库根 `fetch_resources.sh`，那里才有：
#   · paraformer 包 / paraformer.onnx / tokens.txt / vad.onnx 逐文件 sha256 校验
#   · 主源失败自动退镜像（QIBAI_SHERPA_MIRROR）
#   · QIBAI_SKIP_SHERPA / QIBAI_SHERPA_BASE / QIBAI_SHA_CHECK 开关
# 本脚本只做转发，保留旧路径以兼容历史文档与脚本，不再各维护一份逻辑。
#
# 用法：工程根运行  bash scripts/fetch_sherpa_models.sh
#       （会补齐全部缺失资源；已存在的自动跳过，幂等）
# ============================================================
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
echo "[fetch_sherpa_models] 该入口已合并，转发到根目录 fetch_resources.sh（含 sha256 校验与镜像回退）" >&2
exec bash "$ROOT/fetch_resources.sh" "$@"
