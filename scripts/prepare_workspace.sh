#!/usr/bin/env bash
# prepare_workspace.sh — 幂等创建元气小小工作区与 crawler_project 骨架。
# 已存在的目录/文件一律跳过（不覆盖），输出 created=<n> skipped=<m>。
# 默认根 $HOME/Documents/qibai，可用 --root <path> 覆盖。
set -e

ROOT="$HOME/Documents/qibai"
while [ $# -gt 0 ]; do
    case "$1" in
        --root)
            ROOT="$2"
            shift 2
            ;;
        --root=*)
            ROOT="${1#--root=}"
            shift
            ;;
        *)
            echo "unknown arg: $1" >&2
            exit 2
            ;;
    esac
done

CREATED=0
SKIPPED=0

# ensure <path> as dir; counts create/skip
ensure_dir() {
    if [ -d "$1" ]; then
        SKIPPED=$((SKIPPED + 1))
    else
        mkdir -p "$1"
        CREATED=$((CREATED + 1))
    fi
}

# ensure <path> as empty file (only if missing)
ensure_file() {
    if [ -e "$1" ]; then
        SKIPPED=$((SKIPPED + 1))
    else
        : > "$1"
        CREATED=$((CREATED + 1))
    fi
}

ensure_dir "$ROOT"
PROJ="$ROOT/crawler_project"
ensure_dir "$PROJ"
ensure_dir "$PROJ/data"
ensure_dir "$PROJ/logs"

for f in settings fetcher parser pipeline scheduler dedup; do
    ensure_file "$PROJ/$f.py"
done

echo "root=$PROJ"
echo "created=$CREATED skipped=$SKIPPED"
