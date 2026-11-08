#!/usr/bin/env bash
# Assemble frames into a delivery-grade video.
# Usage: make_video.sh <frames-dir> <out.mp4> [fps] [crf] [audio-file]
set -euo pipefail
FRAMES="${1:?usage: make_video.sh <frames-dir> <out.mp4> [fps] [crf]}"
OUT="${2:?}"
FPS="${3:-30}"
CRF="${4:-16}"
AUDIO="${5:-}"

if [ -z "$(ls -A "$FRAMES" 2>/dev/null | head -1)" ]; then echo "no frames in $FRAMES" >&2; exit 1; fi
EXT=jpg; [ -n "$(ls "$FRAMES"/*.png 2>/dev/null | head -1)" ] && EXT=png

# yuv420p + faststart = plays everywhere; bt709 tags keep colours from shifting
if [ -n "$AUDIO" ] && [ -f "$AUDIO" ]; then
  ffmpeg -y -loglevel error -framerate "$FPS" -i "$FRAMES/%05d.$EXT" -i "$AUDIO" \
    -map 0:v:0 -map 1:a:0 -c:v libx264 -preset slow -crf "$CRF" -pix_fmt yuv420p \
    -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
    -vf "scale=trunc(iw/2)*2:trunc(ih/2)*2" -c:a aac -b:a 192k -shortest \
    -movflags +faststart "$OUT"
else
  [ -n "$AUDIO" ] && echo "audio not found: $AUDIO (muxing video only)" >&2
  ffmpeg -y -loglevel error -framerate "$FPS" -i "$FRAMES/%05d.$EXT" \
    -c:v libx264 -preset slow -crf "$CRF" -pix_fmt yuv420p \
    -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
    -vf "scale=trunc(iw/2)*2:trunc(ih/2)*2" -movflags +faststart \
    "$OUT"
fi
echo "video: $OUT  $(du -h "$OUT" | cut -f1)"

# Optional alpha preset (transparent overlay for compositing):
#   make_alpha.sh <frames-dir> <out.webm> <fps>
