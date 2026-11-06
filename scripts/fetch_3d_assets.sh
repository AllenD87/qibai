#!/usr/bin/env bash
# fetch_3d_assets.sh — 《元气小小》3D 三渲二升级：3D 资产下载（幂等）
#
# 阶段一（2026-10-27）调研结论见：
#   /home/user/Doubao/chats/38448289926008322/qibai_3D_资产来源结论_20261027.md
#
# 设计原则：
#   - 幂等：已存在且大小 >0 的文件跳过；重复运行不重复下载。
#   - >50MB 大资产只写真机段落（沙盒不下载）。
#   - itch.io 在国内/沙盒均不可达，标注需真机或代理通道；GitHub 直连可用。
#
# 用法：
#   bash scripts/fetch_3d_assets.sh            # 下载全部（沙盒可达的立即执行）
#   SKIP_LARGE=1 bash scripts/fetch_3d_assets.sh  # 只下载沙盒已验证的小资产

set -u

ASSETS_DIR="${ASSETS_DIR:-$(cd "$(dirname "$0")/.." && pwd)/src/assets/3d}"
mkdir -p "$ASSETS_DIR"

log()  { echo "[fetch_3d] $*"; }
fail() { echo "[fetch_3d][ERROR] $*" >&2; }

# 幂等下载：$1=url $2=输出文件名
dl() {
  local url="$1" out="$2"
  if [[ -s "$out" ]]; then
    log "跳过（已存在）: $out"
    return 0
  fi
  log "下载: $url -> $out"
  curl -fL --retry 3 --connect-timeout 15 -o "$out.tmp" "$url" \
    && mv "$out.tmp" "$out" \
    || { fail "下载失败: $url"; rm -f "$out.tmp"; return 1; }
}

# =============================================================================
# [1/3] Quaternius Universal Animation Library（glTF 版镜像）—— ✅ 已在沙盒验证
#   授权: CC0 1.0（见仓库 LICENSE）
#   内容: Rigify 53 关节骨架 + 46 条动画 clip（idle/talk/walk/sit/dance...）
#         + Mannequin 简模身体（3390+5157 顶点，无 morph，无贴图 → toon 接入干净）
#   体积: zip 3.49MB，解压 9.6MB
#   来源: https://github.com/J-Ponzo/gltf-universal-animation-library
#         （系 Quaternius itch.io 2025-06-10 免费分发版的 glTF 转制镜像）
# =============================================================================
UAL_DIR="$ASSETS_DIR/universal-animation-library"
mkdir -p "$UAL_DIR"
dl "https://github.com/J-Ponzo/gltf-universal-animation-library/archive/refs/heads/main.zip" \
   "$UAL_DIR/ual.zip"

if [[ -s "$UAL_DIR/ual.zip" && ! -d "$UAL_DIR/gltf" ]]; then
  log "解包 ual.zip ..."
  unzip -o -q "$UAL_DIR/ual.zip" -d "$UAL_DIR" \
    && mv "$UAL_DIR/gltf-universal-animation-library-main/glTF" "$UAL_DIR/gltf" \
    && cp "$UAL_DIR/gltf-universal-animation-library-main/LICENSE" "$UAL_DIR/LICENSE" \
    && rm -rf "$UAL_DIR/gltf-universal-animation-library-main" \
    && log "就绪: $UAL_DIR/gltf/AnimationLibrary_Godot_Standard.gltf (+.bin)"
fi

# =============================================================================
# [2/3] Quaternius Ultimate Modular Characters（身体/换装改件）—— ⏸ 待真机
#   授权: CC0 1.0（https://quaternius.com/packs/ultimatemodularcharacters.html）
#   内容: 11 个卡通角色 × 24 动画，每角色拆 4 个可交换部件（同 universal rig）
#   格式: FBX / OBJ / glTF / Blend
#   体积: 约数十 MB（待真机确认）；>50MB 不进沙盒
#   可达性: 官网下载按钮最终跳 itch.io，国内直连不可达。
#   真机步骤（任选其一）：
#     a. 浏览器开 https://quaternius.com/packs/ultimatemodularcharacters.html
#        点 Download（需可访问 itch.io 的网络），把 zip 放到 $UAL_DIR/.. 后解包。
#     b. 若 itch.io 不通，先试官网直连 zip（页面 JS 渲染，需真机抓真实下载 URL）。
#     c. GitHub 暂无完整镜像（beep2bleep/FreeAssetsByKenneyNLandQuaternius
#        仅含 FBX/OBJ 环境包，无 Modular 角色目录），不作为首选。
# =============================================================================
MODULAR_DIR="$ASSETS_DIR/modular-characters"
mkdir -p "$MODULAR_DIR"

if [[ "${SKIP_LARGE:-0}" == "1" ]]; then
  log "SKIP_LARGE=1，跳过大资产段落 [2/3][3/3]。"
else
  # 占位：真机下载命令（确认真实 zip URL 后取消注释）
  # dl "https://quaternius.com/packs/ultimatemodularcharacters.zip" "$MODULAR_DIR/modular.zip"
  # unzip -o -q "$MODULAR_DIR/modular.zip" -d "$MODULAR_DIR"
  log "[2/3] Modular Characters 为大资产，需真机手动下载（见文件头注释）。"
fi

# =============================================================================
# [3/3] Kenney Animated Characters（备选/对比用）—— ⏸ 待真机
#   授权: CC0 1.0（https://kenney-assets.itch.io/animated-characters-3）
#   内容: Human male/female，仅 idle/jump/running 3 条动画（动画量不足，仅作对比）
#   格式: FBX（需 Blender/obj2gltf 转 glTF）
#   真机步骤: 浏览器 https://kenney.nl/assets/series:Animated%20Characters
# =============================================================================
KENNEY_DIR="$ASSETS_DIR/kenney-characters"
mkdir -p "$KENNEY_DIR"

if [[ "${SKIP_LARGE:-0}" != "1" ]]; then
  # 占位（确认 URL 后取消注释）
  # dl "https://kenney.nl/media/pages/assets/animated-characters-3/.../animated-characters-3.zip" \
  #    "$KENNEY_DIR/kenney.zip"
  log "[3/3] Kenney 包需真机从 kenney.nl 下载并转 glTF。"
fi

# =============================================================================
# 路线修正（2026-10-27 用户拍板）：
#   弃 Quaternius 通用 3D 基底（"人物太粗糙"），改 AI 生成 3D 模型路线：
#     - 主路径：Tripo image-to-3D（真机，吃 src/assets/ai_char/<persona>_raw.png，
#       自带 biped mixamo 骨架，导出 GLB 放 src/assets/3d/<persona>.glb）。
#     - 备选：本地 Hunyuan3D-2.1（双 A6000 真机，hf-mirror.com 拉权重）。
#   本脚本下载的 UAL 仅作动画 clip 参考（retarget 用），不再作角色本体。
#   - Mixamo: 中国区 Adobe ID 不可用，不作资产源（仅参考其骨架规范）。
#   - VRoid Studio: Windows/mac 桌面 GUI 人工捏人，不可脚本化。
#
# UAL 转 GLB（可选，已在沙盒验证可加载）：
#   gltf-transform copy $UAL_DIR/gltf/AnimationLibrary_Godot_Standard.gltf \
#                        $ASSETS_DIR/ual_original.glb
# =============================================================================

log "完成。资产目录: $ASSETS_DIR"
