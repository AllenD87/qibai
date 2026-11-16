#!/usr/bin/env bash
# doctor.sh — verify the machine can run the whole pipeline before you burn a render.
# Usage: bash scripts/doctor.sh [--quiet]
set -uo pipefail
QUIET=0; [ "${1:-}" = "--quiet" ] && QUIET=1
ok=0; bad=0
say() { [ "$QUIET" -eq 1 ] || echo "$@"; }
pass() { say "  ok   $1"; ok=$((ok+1)); }
fail() { echo "  MISS $1" >&2; bad=$((bad+1)); }

say "animacode environment check"

# node >= 18
if command -v node >/dev/null 2>&1; then
  major=$(node -p 'process.versions.node.split(".")[0]')
  [ "$major" -ge 18 ] && pass "node $(node -v)" || fail "node $(node -v) (need >= 18)"
else fail "node (need >= 18)"; fi

# chromium
CH="${CHROME:-}"
if [ -n "$CH" ] && [ -x "$CH" ]; then pass "chromium $CH"
else
  for c in chromium chromium-browser google-chrome google-chrome-stable /usr/local/bin/chromium; do
    p=$(command -v "$c" 2>/dev/null) && { pass "chromium $p"; CH="$p"; break; }
  done
  [ -z "$CH" ] && fail "chromium (set CHROME=/path/to/chrome)"
fi

# ffmpeg / ffprobe
command -v ffmpeg >/dev/null 2>&1 && pass "ffmpeg" || fail "ffmpeg"
command -v ffprobe >/dev/null 2>&1 && pass "ffprobe" || fail "ffprobe"

# python + numpy + Pillow
if command -v python3 >/dev/null 2>&1; then
  PYV=$(python3 -c 'import sys; print("%d.%d.%d" % sys.version_info[:3])' 2>/dev/null || echo '?')
  if python3 -c 'import numpy, PIL' >/dev/null 2>&1; then
    pass "python3 $PYV + numpy/pillow"
  else fail "python3 deps (pip install numpy pillow)"; fi
else fail "python3 + numpy + pillow"; fi

# CJK font
FOUND=""
ls /usr/share/fonts/opentype/noto/NotoSansCJK*.ttc >/dev/null 2>&1 && FOUND=1
[ -z "$FOUND" ] && fc-list :lang=zh >/dev/null 2>&1 && FOUND=1
if [ -n "$FOUND" ]; then pass "CJK font (Noto Sans CJK / zh family)"
else say "  warn no CJK font — Chinese text may fall back to a latin font"; fi

say ""
if [ "$bad" -eq 0 ]; then
  say "all set ($ok checks passed). First run: bash scripts/new_scene.sh demo && node scripts/render_frames.mjs --in scene.html --out frames --frames 0-30"
  exit 0
else
  echo "$bad check(s) failed — install the items above, then rerun."
  exit 1
fi
