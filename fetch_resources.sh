#!/usr/bin/env bash
# ============================================================
# 元气小小 · 依赖资源一键补齐脚本（v2）
# ------------------------------------------------------------
# 用途：全量包按项目契约不包含下载型第三方资源（whisper 二进制、
#       ggml 模型、TTS 外部依赖）。本脚本负责补齐：
#   1) whisper.cpp 二进制（whisper-server / whisper-cli，Linux x64/arm64）
#   2) 中文 STT 模型（默认 ggml-small.bin，中文最佳性价比）
#   3) 组装 espeak-ng 离线语音包（tts-espeak/，含中文语音数据；无 root 权限也能装）
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

# ---------- 1. whisper.cpp 二进制（Linux x64/arm64） ----------
#  官方 latest release（v1.9.x）不带二进制资产，资产在带 tag 的构建版里（如 b5454 的
#  whisper-bin-ubuntu-x64.tar.gz）。所以这里扫最近 10 个 release 挑 Linux 资产，
#  再按扩展名走 tar.gz / zip 两条解包路径，最后才退回固定 tag 直链。
if [[ -x "$BIN_DIR/whisper-server" && -x "$BIN_DIR/whisper-cli" ]]; then
  say "whisper-bin 已存在，跳过（$BIN_DIR）"
else
  say "从 whisper.cpp release 定位 Linux 二进制…"
  arch_tag="x64"; [[ "$(uname -m)" == "aarch64" || "$(uname -m)" == "arm64" ]] && arch_tag="arm64"
  REL_JSON=/tmp/whisper_releases.json
  curl -fsSL --max-time 25 "https://api.github.com/repos/ggml-org/whisper.cpp/releases?per_page=10" -o "$REL_JSON" 2>/dev/null || true
  REL_URL="$(ARCH="$arch_tag" python3 - "$REL_JSON" <<'PYEOF' 2>/dev/null
import json, os, sys
arch = os.environ.get("ARCH", "x64")
names = ("ubuntu-"+arch+".tar.gz", "linux-"+arch+".tar.gz", "whisper-bin-"+arch+".zip", "whisper-bin-linux-"+arch+".zip")
try:
    data = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(0)
if not isinstance(data, list):
    sys.exit(0)
for rel in data:                      # 新 release 优先
    assets = {a.get("name"): a.get("browser_download_url") for a in rel.get("assets", [])}
    for n in names:
        if n in assets:
            print(assets[n]); sys.exit(0)
PYEOF
)"
  [[ -z "$REL_URL" ]] && REL_URL="https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-ubuntu-${arch_tag}.tar.gz"
  say "资产：$(basename "$REL_URL")"

  TMPA="$(mktemp /tmp/whisper-bin-XXXXXX)"
  fetch_and_unpack() {
    local url="$1" file="$2"
    curl -fL --max-time 600 "$url" -o "$file" || return 1
    case "$url" in
      *.tar.gz|*.tgz) tar -xzf "$file" -C "$BIN_DIR" ;;
      *.zip)          ( cd "$BIN_DIR" && unzip -o "$file" >/dev/null ) ;;
      *) return 1 ;;
    esac
  }
  if fetch_and_unpack "$REL_URL" "$TMPA"; then
    rm -f "$TMPA"
  else
    rm -f "$TMPA"
    warn "直连下载/解包失败，尝试 ghproxy 镜像…"
    if fetch_and_unpack "https://mirror.ghproxy.com/$REL_URL" "$TMPA"; then
      rm -f "$TMPA"
    else
      rm -f "$TMPA"
      warn "自动获取失败（网络或资产名变动）。手动方案："
      warn "  1) 打开 ${GH_REL} 找带二进制的构建 tag（不是 latest）"
      warn "  2) 下载 whisper-bin-ubuntu-${arch_tag}.tar.gz（或对应 zip），解到 ${BIN_DIR}"
      warn "  3) 或本地编译：cmake -B build && cmake --build build -j"
    fi
  fi
  # release 包内带子目录（whisper-bin-ubuntu-x64/）→ 除二进制外还依赖同目录 libwhisper.so / libggml*.so，
  # 全部文件提回 whisper-bin/ 根，否则运行时报 libwhisper.so.1: cannot open shared object file。
  find "$BIN_DIR" -mindepth 2 -maxdepth 3 \( -type f -o -type l \) -exec mv -f {} "$BIN_DIR"/ \; 2>/dev/null
  find "$BIN_DIR" -mindepth 1 -maxdepth 2 -type d -empty -delete 2>/dev/null
  find "$BIN_DIR" -mindepth 1 -maxdepth 2 -type d -exec rm -rf {} + 2>/dev/null
  chmod +x "$BIN_DIR"/whisper-server "$BIN_DIR"/whisper-cli 2>/dev/null || true
  [[ -x "$BIN_DIR/whisper-server" && -x "$BIN_DIR/whisper-cli" ]] \
    && say "whisper-bin 就绪（$BIN_DIR）" \
    || warn "未找到 whisper-server/whisper-cli，请手动放入 $BIN_DIR"
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

# ---------- 3b. 捆绑 espeak-ng 语音包（离线 TTS；qibai-core 有 6 条单测依赖它） ----------
#  目标布局（qibai-core/src/tts.rs 的 probe_espeak 只认这一种）：
#    tts-espeak/bin/espeak-ng
#    tts-espeak/lib/x86_64-linux-gnu/{libespeak-ng.so.1, libpcaudio.so.0, libsonic.so.0, espeak-ng-data/}
#  Linux x86_64 走 Debian 官方包解包（无需 root/sudo）；macOS/Windows 给出手动放置指引。
ESPEAK_DIR="$ROOT/src-tauri/tts-espeak"
DEB_BASE="${QIBAI_ESPEAK_DEB_BASE:-https://deb.debian.org/debian/pool/main}"
ESPEAK_VER="${QIBAI_ESPEAK_VER:-1.52.0+dfsg-5}"
AUDIO_VER="${QIBAI_PCAUDIO_VER:-1.2-2}"
SONIC_VER="${QIBAI_SONIC_VER:-0.2.0-13+b2}"

espeak_bundle_ok() {
  [[ -x "$ESPEAK_DIR/bin/espeak-ng" && -d "$ESPEAK_DIR/lib/x86_64-linux-gnu/espeak-ng-data" ]]
}

if espeak_bundle_ok; then
  say "espeak-ng 语音包已就位，跳过（$ESPEAK_DIR）"
elif [[ "$(uname -s)/$(uname -m)" == "Linux/x86_64" ]] && command -v dpkg-deb >/dev/null 2>&1; then
  say "组装离线语音包（espeak-ng ${ESPEAK_VER}，Debian 包解包，无需 sudo）…"
  TMPD="$(mktemp -d /tmp/qibai-espeak-XXXX)"
  URLS=(
    "$DEB_BASE/e/espeak-ng/espeak-ng_${ESPEAK_VER}_amd64.deb"
    "$DEB_BASE/e/espeak-ng/libespeak-ng1_${ESPEAK_VER}_amd64.deb"
    "$DEB_BASE/e/espeak-ng/espeak-ng-data_${ESPEAK_VER}_amd64.deb"
    "$DEB_BASE/p/pcaudiolib/libpcaudio0_${AUDIO_VER}_amd64.deb"
    "$DEB_BASE/s/sonic/libsonic0_${SONIC_VER}_amd64.deb"
  )
  ok=1
  for u in "${URLS[@]}"; do
    f="$TMPD/$(basename "$u")"
    curl -fL --max-time 300 "$u" -o "$f" || { warn "下载失败：$u"; ok=0; continue; }
    dpkg-deb -x "$f" "$TMPD/x$(basename "$u" .deb)" || { warn "解包失败：$f"; ok=0; }
  done
  if [[ "$ok" == "1" ]]; then
    mkdir -p "$ESPEAK_DIR/bin" "$ESPEAK_DIR/lib/x86_64-linux-gnu"
    cp -f "$TMPD"/x*/usr/bin/espeak-ng "$ESPEAK_DIR/bin/espeak-ng"
    chmod +x "$ESPEAK_DIR/bin/espeak-ng"
    L="$ESPEAK_DIR/lib/x86_64-linux-gnu"
    cp -a "$TMPD"/x*/usr/lib/x86_64-linux-gnu/libespeak-ng.so.1* "$L"/ 2>/dev/null || true
    cp -a "$TMPD"/x*/usr/lib/x86_64-linux-gnu/libpcaudio.so.0* "$L"/ 2>/dev/null || true
    cp -a "$TMPD"/x*/usr/lib/x86_64-linux-gnu/libsonic.so.0* "$L"/ 2>/dev/null || true
    cp -a "$TMPD"/x*/usr/lib/x86_64-linux-gnu/espeak-ng-data "$L"/ 2>/dev/null || true
    # 版本化 soname 补短名软链（LD_LIBRARY_PATH 里必须能找到 .so.1/.so.0）
    for pair in "libespeak-ng.so.1:libespeak-ng.so.1.1.51" "libpcaudio.so.0:libpcaudio.so.0.0.1" "libsonic.so.0:libsonic.so.0.2.0"; do
      want="${pair%%:*}"; real="${pair##*:}"
      [[ -e "$L/$want" ]] || { [[ -e "$L/$real" ]] && ln -sf "$real" "$L/$want"; }
    done
    if espeak_bundle_ok && LD_LIBRARY_PATH="$L" "$ESPEAK_DIR/bin/espeak-ng" --version >/dev/null 2>&1; then
      say "语音包组装完成并可运行（$(du -sh "$ESPEAK_DIR" | cut -f1)，$ESPEAK_DIR）"
    else
      warn "语音包解包后自检未通过，请检查 $ESPEAK_DIR（可能缺 libpcaudio/libsonic）"
    fi
  else
    warn "自动组装失败（网络或包版本变动）。手动方案："
    warn "  · Debian/Ubuntu：sudo apt install espeak-ng libespeak-ng1，然后"
    warn "    mkdir -p $ESPEAK_DIR/bin $ESPEAK_DIR/lib/x86_64-linux-gnu"
    warn "    cp /usr/bin/espeak-ng $ESPEAK_DIR/bin/ && cp -a /usr/lib/x86_64-linux-gnu/libespeak-ng.so.1* /usr/lib/x86_64-linux-gnu/libpcaudio.so.0* /usr/lib/x86_64-linux-gnu/libsonic.so.0* /usr/lib/x86_64-linux-gnu/espeak-ng-data $ESPEAK_DIR/lib/x86_64-linux-gnu/"
  fi
  rm -rf "$TMPD"
else
  warn "当前平台不是 Linux x86_64，语音包需手动放置：$ESPEAK_DIR"
  warn "  · 目录结构：bin/espeak-ng + lib/x86_64-linux-gnu/{libespeak-ng*,espeak-ng-data}"
  warn "  · 或设 QIBAI_TTS_ESPEAK_DIR 指向已有的 espeak-ng 目录后重启应用"
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
espeak_bundle_ok || missing="$missing tts-espeak(espeak-ng 语音包)"
if [[ -n "$missing" ]]; then
  warn "以下资源仍未就位：$missing"
  warn "  · 重跑： bash fetch_resources.sh（已成功的部分会自动跳过）"
  warn "  · 或手动放入：二进制 → $BIN_DIR ；模型 → $MODEL_DIR"
  exit 1
fi
echo "  ls -lh $BIN_DIR/whisper-server $MODEL_DIR/$pick_model"
echo "  启动应用后日志应显示：audio stt: whisper=... model=... （OK）"
