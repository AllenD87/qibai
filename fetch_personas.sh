#!/usr/bin/env bash
# fetch_personas.sh —— 拉取「元气小小」五角色全量立绘与动作帧
#
# 背景：整套美术帧 631 MB，放普通 git 会让 clone 永久臃肿，放 Git LFS 会吃掉免费配额。
# 所以仓库只放预览图（docs/images/personas/，合计 < 2 MB），全量帧托管在 GitHub Release。
#
# 用法（在仓库根目录执行）：
#   bash fetch_personas.sh            # 下载 → 校验 → 解压到 src/assets/personas/
#   bash fetch_personas.sh --force    # 已存在也重下重装
#   QIBAI_PERSONAS_TAG=v0.2.0 bash fetch_personas.sh   # 指定别的版本
#
# 依赖：curl、unzip；校验值优先用 md5sum（Linux）/ md5（macOS），两者都没有则跳过校验并提示。

set -euo pipefail

OWNER_REPO="AllenD87/qibai"
TAG="${QIBAI_PERSONAS_TAG:-v0.1.0}"
BASE="personas_frames_${TAG}"
PARTS=(0 1 2)
PART_MD5=(
  "490606a16d885d76c5e9cf6ac929838f"
  "c9aca441173567297fc1e629b195a9bb"
  "1bf02aa4abb6cdcbbe994396c900c9a6"
)
PART_SIZE=(
  "167772160"
  "167772160"
  "164762447"
)
ZIP_MD5="e6aafa6ff36334c15b7cb07c921510f2"   # 拼接后整包
ZIP_SIZE=500306767

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEST="$ROOT/src/assets/personas"
TMP="$ROOT/.personas_download"
FORCE=0
[ "${1:-}" = "--force" ] && FORCE=1

need() { command -v "$1" >/dev/null 2>&1 || { echo "缺少依赖：$1" >&2; exit 1; }; }
need curl
need unzip

md5_of() {
  if command -v md5sum >/dev/null 2>&1; then md5sum "$1" | awk '{print $1}'
  elif command -v md5 >/dev/null 2>&1; then md5 -q "$1"
  else echo "skip"; fi
}
size_of() {
  if stat -f%z "$1" >/dev/null 2>&1; then stat -f%z "$1"; else stat -c%s "$1"; fi
}

if [ -d "$DEST/gudian" ] && [ "$FORCE" -eq 0 ]; then
  echo "已存在：$DEST"
  echo "如需重新下载并覆盖，加 --force。"
  exit 0
fi

mkdir -p "$TMP"
URL_BASE="https://github.com/$OWNER_REPO/releases/download/$TAG"

echo "==> 下载 $BASE 分卷（共 3 卷，约 478 MB，来自 $TAG）"
for i in "${PARTS[@]}"; do
  f="$TMP/$BASE.zip.part$i"
  expected="${PART_SIZE[$i]}"
  if [ -f "$f" ] && [ "$(size_of "$f")" = "$expected" ]; then
    echo "    分卷 $i 已存在，跳过下载"
  else
    echo "    下载 part$i ..."
    curl -fL --retry 3 --retry-delay 2 -C - -o "$f" "$URL_BASE/$BASE.zip.part$i" \
      || { echo "下载失败：$URL_BASE/$BASE.zip.part$i" >&2
           echo "如果这是 404，说明该版本 Release 还没上传素材包；到 https://github.com/$OWNER_REPO/releases 核对。" >&2
           exit 1; }
  fi
  m="$(md5_of "$f")"
  if [ "$m" != "skip" ] && [ "$m" != "${PART_MD5[$i]}" ]; then
    echo "分卷 $i 校验失败（期望 ${PART_MD5[$i]}，实际 $m），已删除重跑即可重下" >&2
    rm -f "$f"; exit 1
  fi
done

echo "==> 拼接并校验整包"
cat "$TMP/$BASE.zip.part"* > "$TMP/$BASE.zip"
s="$(size_of "$TMP/$BASE.zip")"
[ "$s" = "$ZIP_SIZE" ] || { echo "整包大小不符（期望 $ZIP_SIZE，实际 $s）" >&2; exit 1; }
m="$(md5_of "$TMP/$BASE.zip")"
[ "$m" = "skip" ] || [ "$m" = "$ZIP_MD5" ] || { echo "整包校验失败（期望 $ZIP_MD5，实际 $m）" >&2; exit 1; }

echo "==> 解压到 src/assets/personas/"
mkdir -p "$DEST"
unzip -q -o "$TMP/$BASE.zip" -d "$ROOT/src/assets"

echo "==> 完成，目录结构："
find "$DEST" -maxdepth 2 -type d | sed "s|$ROOT/||" | sort | head -20
echo
echo "帧数：$(find "$DEST" -name '*.png' | wc -l | tr -d ' ') 张"
echo "现在可以直接 npm run dev / cargo tauri dev 启动，前端按 <角色>/<动作>/<帧>.png 路径加载。"

if [ "${KEEP_ZIP:-0}" != "1" ]; then
  rm -rf "$TMP"
  echo "（临时分卷已清理；想留包可加 KEEP_ZIP=1）"
else
  echo "整包与分卷保留在 $TMP"
fi
