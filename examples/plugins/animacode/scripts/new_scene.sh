#!/usr/bin/env bash
# Create a new animation project from the animacode template.
# Usage: new_scene.sh <output-dir> [fps] [duration] [width] [height]
set -euo pipefail
SKILL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:?usage: new_scene.sh <output-dir>}"
FPS="${2:-30}"; DUR="${3:-8}"; W="${4:-1920}"; H="${5:-1080}"

mkdir -p "$OUT"
cp "$SKILL_DIR/assets/runtime/animacore.js" "$OUT/animacore.js"
cp "$SKILL_DIR/assets/template/scene.html" "$OUT/scene.html"
python3 - "$OUT/scene.html" "$FPS" "$DUR" "$W" "$H" <<'PY'
import re, sys
p, fps, dur, w, h = sys.argv[1:6]
s = open(p, encoding='utf-8').read()
s = re.sub(r'const W = \d+, H = \d+, FPS = \d+, DURATION = \d+;',
           f'const W = {int(w)}, H = {int(h)}, FPS = {int(fps)}, DURATION = {float(dur)};', s)
open(p, 'w', encoding='utf-8').write(s)
PY
echo "created $OUT/scene.html  ${W}x${H} @${FPS}fps ${DUR}s"
