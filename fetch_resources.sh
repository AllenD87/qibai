#!/usr/bin/env bash
# ============================================================
# 元气小小 · 依赖资源一键补齐脚本（v1）
# ------------------------------------------------------------
# 用途：全量包按项目契约不包含下载型第三方资源（whisper 二进制、
#       ggml 模型、TTS 外部依赖）。本脚本负责补齐：
#   1) whisper.cpp 二进制（whisper-server / whisper-cli，Linux x64）
#   2) 中文 STT 模型（默认 ggml-small.bin，中文最佳性价比）
#   3) 检查 espeak-ng（本地 TTS 兜底，缺失时给出安装命令）
#   4) 可选：piper 中文离线模型（edge-tts 在线不可用时的本地女声）
# 用法：解压全量包后，在工程根运行：  bash fetch_resources.sh
# 可重跑：已下载的部分自动跳过（幂等）。
# ============================================================
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$ROOT/src-tauri/whisper-bin"
MODEL_DIR="$ROOT/src-tauri/models"
mkdir -p "$BIN_DIR" "$MODEL_DIR"

HF="https://hf-mirror.com/ggerganov/whisper.cpp/resolve/main"
GH_API="https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest"
GH_REL="https://github.com/ggml-org/whisper.cpp/releases"

say()  { printf '\033[1;32m[资源]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[提示]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[失败]\033[0m %s\n' "$*"; exit 1; }

# ---------- 1. whisper.cpp 二进制（Linux x64） ----------
if [[ -x "$BIN_DIR/whisper-server" && -x "$BIN_DIR/whisper-cli" ]]; then
  say "whisper-bin 已存在，跳过（$BIN_DIR）"
else
  say "尝试从 whisper.cpp 官方 release 下载 Linux x64 二进制…"
  TMPZ="$(mktemp /tmp/whisper-bin-XXXX.zip)"
  ZIP_URL=""
  if curl -fsSL --max-time 20 "$GH_API" -o /tmp/whisper_latest.json 2>/dev/null; then
    ZIP_URL="$(python3 -c "
import json
try:
    d=json.load(open('/tmp/whisper_latest.json'))
    print(next((a['browser_download_url'] for a in d.get('assets',[]) if 'whisper-bin-x64' in a['name'] and a['name'].endswith('.zip')),''))
except Exception:
    print('')" 2>/dev/null)"
  fi
  if [[ -z "$ZIP_URL" ]]; then
    ZIP_URL="$GH_REL/download/latest/whisper-bin-x64.zip"
  fi
  if curl -fL --max-time 600 "$ZIP_URL" -o "$TMPZ"; then
    ( cd "$BIN_DIR" && unzip -o "$TMPZ" && rm -f "$TMPZ" ) \
      || { rm -f "$TMPZ"; warn "解压失败，请手动下载"; }
    # v2：release zip 可能带子目录（whisper-bin-x64/…）→ 把二进制提回 whisper-bin/ 根
    find "$BIN_DIR" -maxdepth 3 -type f \( -name whisper-cli -o -name whisper-server \) -exec mv -f {} "$BIN_DIR"/ \; 2>/dev/null
    chmod +x "$BIN_DIR"/whisper-server "$BIN_DIR"/whisper-cli 2>/dev/null || true
    if [[ -x "$BIN_DIR/whisper-server" && -x "$BIN_DIR/whisper-cli" ]]; then
      say "whisper-bin 下载完成（$BIN_DIR）"
    else
      warn "解压后未找到 whisper-server/whisper-cli，请手动将二进制放入 $BIN_DIR"
    fi
  else
    rm -f "$TMPZ"
    warn "直连下载失败（网络/资产名变动），尝试国内镜像 ghproxy…"
    if curl -fL --max-time 600 "https://mirror.ghproxy.com/$ZIP_URL" -o "$TMPZ"; then
      ( cd "$BIN_DIR" && unzip -o "$TMPZ" && rm -f "$TMPZ" ) \
        || { rm -f "$TMPZ"; warn "镜像解压失败，请手动下载"; }
      find "$BIN_DIR" -maxdepth 3 -type f \( -name whisper-cli -o -name whisper-server \) -exec mv -f {} "$BIN_DIR"/ \; 2>/dev/null
      chmod +x "$BIN_DIR"/whisper-server "$BIN_DIR"/whisper-cli 2>/dev/null || true
      if [[ -x "$BIN_DIR/whisper-server" && -x "$BIN_DIR/whisper-cli" ]]; then
        say "whisper-bin 镜像下载完成（$BIN_DIR）"
      else
        warn "镜像解压后未找到 whisper-server/whisper-cli，请手动将二进制放入 $BIN_DIR"
      fi
    else
      rm -f "$TMPZ"
      warn "自动下载失败（网络或资产名变动）。手动获取："
      warn "  1) 打开 ${GH_REL}"
      warn "  2) 下载 Linux x64 资产（whisper-bin-x64.zip，含 whisper-server + whisper-cli）"
      warn "  3) 解压到 ${BIN_DIR} 并 chmod +x"
      warn "  · 或本地编译：工程内 whisper/ 源码 → cmake -B build && cmake --build build -j"
    fi
  fi
fi

# ---------- 2. 中文 STT 模型 ----------
pick_model="${QIBAI_WHISPER_MODEL:-ggml-small.bin}"
if [[ -f "$MODEL_DIR/$pick_model" ]]; then
  say "模型 $pick_model 已存在，跳过"
else
  say "下载中文 STT 模型 ${pick_model}（small≈466MB 中文最佳；可设 QIBAI_WHISPER_MODEL=ggml-base.bin 换 base≈142MB）…"
  if curl -fL --max-time 1800 "$HF/$pick_model" -o "$MODEL_DIR/$pick_model.tmp"; then
    mv "$MODEL_DIR/$pick_model.tmp" "$MODEL_DIR/$pick_model"
    say "模型下载完成：$MODEL_DIR/$pick_model"
  else
    rm -f "$MODEL_DIR/$pick_model.tmp"
    die "模型下载失败：$HF/$pick_model（检查网络后重跑本脚本）"
  fi
fi

# ---------- 3. espeak-ng（本地 TTS 兜底） ----------
if command -v espeak-ng >/dev/null 2>&1 || command -v espeak >/dev/null 2>&1; then
  say "espeak-ng 系统可用 ✓"
else
  warn "系统缺少 espeak-ng（本地 TTS 兜底）。Debian/Ubuntu 安装： sudo apt install espeak-ng"
fi

# ---------- 4.（可选）piper 中文离线模型 ----------
if [[ ! -f "$MODEL_DIR/zh_CN-huayan-medium.onnx" ]]; then
  warn "可选：本地离线 TTS（piper）模型未安装。如需离线女声："
  warn "  curl -L -o $MODEL_DIR/zh_CN-huayan-medium.onnx https://hf-mirror.com/rhasspy/piper-voices/resolve/main/zh/zh_CN/huayan/medium/zh_CN-huayan-medium.onnx"
fi

# ---------- 完成（自检） ----------
say "资源检查完成。"
missing=""
[[ -x "$BIN_DIR/whisper-server" ]] || missing="$missing whisper-server"
[[ -x "$BIN_DIR/whisper-cli" ]]   || missing="$missing whisper-cli"
[[ -f "$MODEL_DIR/$pick_model" ]] || missing="$missing $pick_model"
if [[ -n "$missing" ]]; then
  warn "以下资源仍未就位：$missing"
  warn "  · 重跑： bash fetch_resources.sh（已成功的部分会自动跳过）"
  warn "  · 或手动放入：二进制 → $BIN_DIR ；模型 → $MODEL_DIR"
  exit 1
fi
echo "  ls -lh $BIN_DIR/whisper-server $MODEL_DIR/$pick_model"
echo "  启动应用后日志应显示：audio stt: whisper=... model=... （OK）"
