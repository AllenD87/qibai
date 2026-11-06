#!/usr/bin/env bash
# ============================================================
# 元气小小 · sherpa-onnx 中文 STT（Paraformer-zh）资源补齐脚本
# ------------------------------------------------------------
# 用途：STT 增量包不包含 sherpa-onnx 运行时与模型（数百 MB 级）。
#       本脚本负责补齐，解压后 Rust 侧按 src-tauri/models/sherpa/ 下的
#       固定文件名查找（见 qibai-core/src/sherpa_engine.rs）：
#         paraformer.onnx  tokens.txt  vad.onnx  punc.onnx
# 用法：工程根运行  bash scripts/fetch_sherpa_models.sh
# 可重跑：已存在的文件自动跳过（幂等）。
# 网络：GitHub 走 gh-proxy.com 前缀（沙盒/内网无 GitHub 直连时）。
#
# 模型体积（下载 ≈ 解压后量级）：
#   paraformer-zh int8  : ~233 MB（离线中文识别主模型）
#   ct-punc (fp32)     : ~281 MB（标点恢复；该 bundle 仅 fp32）
#   silero VAD         : ~0.6 MB（端点检测）
#   合计下载约 ~520 MB
# 运行时依赖：python3 + `pip install -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com sherpa-onnx`（阿里云镜像，onnxruntime 随 wheel 捆绑）。
# ============================================================
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/src-tauri/models/sherpa"
mkdir -p "$DEST"

GH="https://gh-proxy.com/https://github.com/k2-fsa/sherpa-onnx/releases/download"
PF_TGZ="$GH/asr-models/sherpa-onnx-paraformer-zh-2023-09-14.tar.bz2"
VAD_ONNX="$GH/asr-models/silero_vad.onnx"
PUNC_TGZ="$GH/punctuation-models/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12.tar.bz2"

say()  { printf '\033[1;36m[sherpa]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[提示]\033[0m %s\n' "$*"; }

# ---------- 0. python 运行时 ----------
if ! command -v python3 >/dev/null 2>&1; then
  warn "未找到 python3，请先安装（Ubuntu: sudo apt install python3）"
fi
say "安装/升级 sherpa-onnx（含 onnxruntime）…"
pip3 install --upgrade sherpa-onnx 2>&1 | tail -2 || warn "pip 安装 sherpa-onnx 失败，请手动执行：pip3 install sherpa-onnx"

TMPD="$(mktemp -d)"; trap 'rm -rf "$TMPD"' EXIT

# ---------- 1. paraformer int8 + tokens ----------
if [[ -s "$DEST/paraformer.onnx" && -s "$DEST/tokens.txt" ]]; then
  say "paraformer.onnx / tokens.txt 已存在，跳过"
else
  say "下载 paraformer-zh int8（~233MB）…"
  curl -fL --retry 3 --max-time 1800 -o "$TMPD/pf.tar.bz2" "$PF_TGZ" \
    && tar xjf "$TMPD/pf.tar.bz2" -C "$TMPD" \
    && cp "$TMPD/sherpa-onnx-paraformer-zh-2023-09-14/model.int8.onnx" "$DEST/paraformer.onnx" \
    && cp "$TMPD/sherpa-onnx-paraformer-zh-2023-09-14/tokens.txt" "$DEST/tokens.txt" \
    && say "paraformer.onnx + tokens.txt 就绪" \
    || warn "paraformer 下载/解压失败，请重试或手动下载 $PF_TGZ"
fi

# ---------- 2. VAD ----------
if [[ -s "$DEST/vad.onnx" ]]; then
  say "vad.onnx 已存在，跳过"
else
  say "下载 silero VAD（~0.6MB）…"
  curl -fL --retry 3 --max-time 300 -o "$DEST/vad.onnx" "$VAD_ONNX" \
    && say "vad.onnx 就绪" || warn "vad 下载失败：$VAD_ONNX"
fi

# ---------- 3. ct-punc ----------
if [[ -s "$DEST/punc.onnx" ]]; then
  say "punc.onnx 已存在，跳过"
else
  say "下载 ct-punc（~281MB）…"
  curl -fL --retry 3 --max-time 1800 -o "$TMPD/punc.tar.bz2" "$PUNC_TGZ" \
    && tar xjf "$TMPD/punc.tar.bz2" -C "$TMPD" \
    && cp "$TMPD/sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12/model.onnx" "$DEST/punc.onnx" \
    && say "punc.onnx 就绪" \
    || warn "punc 下载/解压失败，请重试或手动下载 $PUNC_TGZ"
fi

# ---------- 4. runner 脚本自检 ----------
say "模型目录：$DEST"
ls -la "$DEST"
if [[ -s "$DEST/paraformer.onnx" && -s "$DEST/tokens.txt" && -s "$DEST/vad.onnx" && -s "$DEST/punc.onnx" ]]; then
  say "全部就绪。切换：export STT_ENGINE=paraformer（=whisper 回退；不设则资源齐全时自动用 paraformer）"
else
  warn "仍有缺失文件，见上方日志。"
fi
