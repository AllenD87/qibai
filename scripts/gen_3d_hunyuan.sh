#!/usr/bin/env bash
# ============================================================
# ⚠️ 已停用（用户改向 2026-10-27：不做真 3D/GLB，改 3D 国漫 CG
#    风格立绘主展示）——保留备查，勿再使用。
# ============================================================
# gen_3d_hunyuan.sh — 真机双 A6000：Hunyuan3D-2.1 image-to-3D 本地生成
#
# ⚠️ 真机执行（沙盒无 GPU/网络受限，本脚本不在沙盒跑）。
# 前提：Ubuntu 22.04+ / 双卡 A6000 24GB / CUDA 12.4 / Python 3.10+
# 用法：
#   PERSONA=huopo bash scripts/gen_3d_hunyuan.sh
#   PERSONA=all   bash scripts/gen_3d_hunyuan.sh     # 批量 5 形象
#
# 产物：src/assets/3d/<persona>.glb（A 组 3d_character.js 按此路径加载）
# 授权：Hunyuan3D 权重开源可商用（见 HF LICENSE）；输入图为自制 AI 立绘。

set -euo pipefail

QIBAI_ROOT="${QIBAI_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$QIBAI_ROOT"

PERSONA="${PERSONA:-huopo}"
GEN_DIR="${GEN_DIR:-$QIBAI_ROOT/_3d_gen/hunyuan}"
OUT_DIR="$QIBAI_ROOT/src/assets/3d"
INPUT_IMG="$QIBAI_ROOT/src/assets/ai_char/${PERSONA}_raw.png"

# HF 国内镜像（强制走 hf-mirror，避免 huggingface.co 超时）
export HF_ENDPOINT="${HF_ENDPOINT:-https://hf-mirror.com}"

# 5 形象白名单
ALL_PERSONAS="gudian dushi wenrou huopo reqing"

log()  { echo "[hunyuan3d] $*"; }
fail() { echo "[hunyuan3d][ERROR] $*" >&2; exit 1; }

[[ -f "$INPUT_IMG" ]] || fail "输入图不存在: $INPUT_IMG（先跑 2D 立绘生成）"
command -v python3 >/dev/null || fail "需要 python3"
nvidia-smi >/dev/null 2>&1 || fail "未检测到 GPU（nvidia-smi 失败）——本脚本仅在真机跑"

# ---------- 1. 仓库与依赖 ----------
mkdir -p "$GEN_DIR"
if [[ ! -d "$GEN_DIR/Hunyuan3D-2" ]]; then
  log "克隆 Tencent/Hunyuan3D-2 仓库 ..."
  git clone --depth 1 https://hf-mirror.com/Tencent/Hunyuan3D-2 "$GEN_DIR/Hunyuan3D-2"
fi
cd "$GEN_DIR/Hunyuan3D-2"

if [[ ! -d ".venv" ]]; then
  log "建 venv 并装依赖（首次约 5-10 分钟）..."
  python3 -m venv .venv
  # shellcheck disable=SC1091
  source .venv/bin/activate
  pip install --upgrade pip -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com
  # torch cu124 必须走 PyTorch 官方 CUDA wheel 源（阿里镜像无 cu124 完整版），保持官方 index
  pip install torch==2.4.0 torchvision --index-url https://download.pytorch.org/whl/cu124
  # 其余依赖统一阿里云镜像；若 requirements 强制重装 torch 出现 CPU 版，可去掉本行 -i 回退官方源
  pip install -r requirements.txt -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com
  pip install pygltflib trimesh rembg -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com 2>/dev/null || true
else
  # shellcheck disable=SC1091
  source .venv/bin/activate
fi

# ---------- 2. 权重下载（hf-mirror 自动缓存；首次约 30-40GB） ----------
# 主权重：hunyuan3d-dit-v2-1/model.fp16.ckpt (~12GB)
#         hunyuan3d-vae-v2-1/model.fp16.ckpt (~0.5GB)
#         hunyuan3d-paintpbr-v2-1/* (~10GB)
log "预下载权重（huggingface_hub 走 hf-mirror）..."
python3 - <<'PY'
from huggingface_hub import snapshot_download
snapshot_download(repo_id="tencent/Hunyuan3D-2.1",
                  local_dir="./weights",
                  allow_patterns=["hunyuan3d-dit-v2-1/*",
                                   "hunyuan3d-vae-v2-1/*",
                                   "hunyuan3d-paintpbr-v2-1/*",
                                   "*.md"])
PY

# ---------- 3. image-to-3D 推理 ----------
run_one() {
  local p="$1"
  local img="$QIBAI_ROOT/src/assets/ai_char/${p}_raw.png"
  local work="$GEN_DIR/out_$p"
  mkdir -p "$work"
  [[ -f "$img" ]] || { log "跳过 $p（无输入图 $img）"; return 1; }

  log "[$p] 形状生成（image2shape，A6000 约 3-6 分钟）..."
  python3 gen_image2shape.py \
    --pretrained "./weights" \
    --input_image "$img" \
    --output_dir "$work" \
    --octree_resolution 256 \
    --num_slots 8000 \
    --mc_level 0.0 \
    --perf_mode  # 2.1 默认 fp16

  log "[$p] PBR 贴图生成（paintpbr，约 2-4 分钟）..."
  python3 gen_pbr.py \
    --pretrained "./weights" \
    --input_mesh "$work/mesh.obj" \
    --input_image "$img" \
    --output_dir "$work"

  # 转 GLB（trimesh 一把梭；带贴图）
  log "[$p] mesh → GLB ..."
  python3 - <<PY
import trimesh
m = trimesh.load("$work/mesh.obj")
if hasattr(m, "geometry"):
    m = trimesh.util.concatenate(list(m.geometry.values()))
m.export("$work/${p}_raw.glb")
PY

  # ---------- 4. gltf-transform 瘦身（简化+去冗余） ----------
  log "[$p] gltf-transform 瘦身 ..."
  if command -v gltf-transform >/dev/null; then
    gltf-transform simplify "$work/${p}_raw.glb" "$work/${p}_slim.glb" --error 0.005 || cp "$work/${p}_raw.glb" "$work/${p}_slim.glb"
    gltf-transform prune "$work/${p}_slim.glb" "$work/${p}_final.glb"
  else
    cp "$work/${p}_raw.glb" "$work/${p}_final.glb"
  fi

  mkdir -p "$OUT_DIR"
  mv "$work/${p}_final.glb" "$OUT_DIR/${p}.glb"
  log "[$p] 完成 -> $OUT_DIR/${p}.glb ($(du -h "$OUT_DIR/${p}.glb" | cut -f1))"
}

if [[ "$PERSONA" == "all" ]]; then
  for p in $ALL_PERSONAS; do run_one "$p"; done
else
  run_one "$PERSONA"
fi

log "全部完成。产物在 $OUT_DIR/，交 A 组 3d_character.js 加载冒烟。"
