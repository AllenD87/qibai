#!/usr/bin/env bash
# ============================================================
# 元气小小 · sherpa-onnx 模型补齐脚本（≈233MB，不进增量包）
#   paraformer-zh int8  主识别模型
#   silero VAD         端点检测
# 标点模型 ct-punc（≈281MB）为可选：冒烟/联调不需要，需要时手补。
# 网络：GitHub 走 gh-proxy.com 前缀。幂等：已存在即跳过。
# 注意：权威入口是根目录 `fetch_resources.sh`（含 sha256 校验与镜像回退），本脚本为旧入口。
# ============================================================
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/src-tauri/models/sherpa"
mkdir -p "$DEST"
GH="https://gh-proxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download"
cd "$DEST"

if [ ! -f silero_vad.onnx ]; then
  echo "[sherpa] 下载 silero VAD (~0.6MB)"
  curl -sL -o silero_vad.onnx "$GH/asr-models/silero_vad.onnx"
fi

if [ ! -f paraformer.onnx ]; then
  echo "[sherpa] 下载 paraformer-zh int8 (~233MB)"
  curl -sL -o paraformer.tgz "$GH/asr-models/sherpa-onnx-paraformer-zh-2023-09-14.tar.bz2"
  tar xjf paraformer.tgz
  cp sherpa-onnx-paraformer-zh-2023-09-14/model.int8.onnx paraformer.onnx
  cp sherpa-onnx-paraformer-zh-2023-09-14/tokens.txt tokens.txt
  rm -rf paraformer.tgz sherpa-onnx-paraformer-zh-2023-09-14
fi
echo "[sherpa] 完成：$(du -sh "$DEST" | cut -f1)"
