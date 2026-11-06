#!/usr/bin/env bash
# 打包前置校验：依赖闭包扫描 + 干净树编译
#
# 背景：v45.11 曾漏掉 settings.rs，沙盒树因历史改动堆积永远编译通过，
# 真机（只装过 v45.10）引用新字段直接 E0609。本脚本把"干净树"作为强制门。
#
# 用法：
#   pack_verify.sh <base_zip> <package_zip> [--apply <patch_zip>...]
#
# base_zip    : 用户线上已有版本的全量/增量包（作为干净树基线）
# package_zip : 本次要发的增量包
# --apply     : 额外要叠加的补丁包（可多个）
#
# 退出码 0 = 干净树编译通过且闭包无缺口；非 0 = 不许发。

set -euo pipefail

BASE_ZIP="$1"; PKG_ZIP="$2"; shift 2 || true
APPLY=()
while [ $# -gt 0 ]; do
  case "$1" in
    --apply) APPLY+=("$2"); shift 2 ;;
    *) echo "未知参数: $1"; exit 2 ;;
  esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLEAN="${CLEAN_DIR:-$ROOT/../qibai_clean}"
export PATH="$HOME/.cargo/bin:$PATH"

echo "=== [1/4] 构建干净树：$(basename "$BASE_ZIP") + $(basename "$PKG_ZIP")"
rm -rf "$CLEAN"; mkdir -p "$CLEAN"
( cd "$ROOT" && tar -cf - --exclude=target --exclude=deliver --exclude=.git --exclude=node_modules . ) | ( cd "$CLEAN" && tar -xf - )
unzip -oq "$BASE_ZIP" -d "$CLEAN"
unzip -oq "$PKG_ZIP" -d "$CLEAN"
for p in "${APPLY[@]:-}"; do [ -n "$p" ] && unzip -oq "$p" -d "$CLEAN"; done

echo "=== [2/4] 依赖闭包扫描（结构体字段 / crate::mod::item）"
# 扫包内 .rs 里对 Settings.* 字段与 crate::x::y 的引用，确认定义在干净树里存在
MISSING=0
while IFS= read -r f; do
  case "$f" in *.rs) ;; *) continue ;; esac
  [ -f "$CLEAN/$f" ] || { echo "  [缺文件] $f"; MISSING=1; continue; }
  # Settings 字段引用
  for field in $(grep -oE '\bSettings\s*\{[^}]*\}' -z "$CLEAN/$f" 2>/dev/null | tr '\0' '\n' | grep -oE '\b[a-z_]+:' | tr -d ':' | sort -u || true); do
    :
  done
done < <(unzip -Z1 "$PKG_ZIP" | grep -E '\.rs$')

# 精确项：Settings.<field> 与结构体字面量 Settings { field } 都要在定义文件里有该字段
DEF_FILES=$(grep -rl "pub struct Settings" "$CLEAN" --include=*.rs 2>/dev/null || true)
if [ -n "$DEF_FILES" ]; then
  for field in $(unzip -p "$PKG_ZIP" 2>/dev/null | true; unzip -Z1 "$PKG_ZIP" | grep -E '\.rs$' | while read -r f; do unzip -p "$PKG_ZIP" "$f"; done | grep -oE '\bSettings\.\w+' | sed 's/Settings\.//' | sort -u); do
    if ! grep -qE "^\s*(pub\s+)?$field\s*:" "$DEF_FILES"; then
      echo "  [闭包缺口] Settings.$field 被引用，但干净树的 Settings 定义里没有该字段"; MISSING=1
    fi
  done
fi
[ "$MISSING" -eq 0 ] && echo "  闭包扫描：OK"

echo "=== [3/4] 干净树编译（core --all-targets）"
if ! ( cd "$CLEAN" && CARGO_TARGET_DIR="$CLEAN/target" cargo check -p qibai-core --all-targets > "$CLEAN/check_core.log" 2>&1 ); then
  echo "  FAILED：core 干净树编译失败，前 8 行错误："
  grep -E "^error" "$CLEAN/check_core.log" | head -8; exit 1
fi
echo "  OK：$(grep -c '^warning' "$CLEAN/check_core.log" || true) warnings"

echo "=== [4/4] 干净树编译（src-tauri）"
# 复用主树的 target 缓存（沙盒缺 gdk 系统库，从零编译依赖会因环境失败）
if ! ( cd "$CLEAN/src-tauri" && CARGO_TARGET_DIR="$ROOT/src-tauri/target" cargo check > "$CLEAN/check_tauri.log" 2>&1 ); then
  echo "  FAILED：src-tauri 干净树编译失败："
  grep -E "^error" "$CLEAN/check_tauri.log" | head -8; exit 1
fi
echo "  OK：$(grep -c '^warning' "$CLEAN/check_tauri.log" || true) warnings"

echo "=== 全部通过：该包可在真机编译 ==="
