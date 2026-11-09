#!/usr/bin/env bash
# Assemble frames into a delivery-grade video.
# Usage: make_video.sh <frames-dir> <out.mp4> [fps] [crf] [audio-file] [duration]
#
#   duration  optional, seconds. When given, the frame count and the audio length
#             are both checked against it, so a stale tail from an older (longer)
#             cut cannot slip into the render.
set -euo pipefail
FRAMES="${1:?usage: make_video.sh <frames-dir> <out.mp4> [fps] [crf] [audio] [duration]}"
OUT="${2:?}"
FPS="${3:-30}"
CRF="${4:-16}"
AUDIO="${5:-}"
DUR="${6:-}"

if [ -z "$(ls -A "$FRAMES" 2>/dev/null | head -1)" ]; then echo "no frames in $FRAMES" >&2; exit 1; fi
EXT=jpg; [ -n "$(ls "$FRAMES"/*.png 2>/dev/null | head -1)" ] && EXT=png

# ---- frame sequence sanity: ffmpeg stops silently at the first gap, and
# leftover frames from a longer previous cut silently extend the video.
read -r FIRST LAST COUNT <<<"$(ls "$FRAMES"/*."$EXT" | sed 's/.*\///' | cut -c1-5 | sort -n | awk 'NR==1{f=$1} {l=$1; n++} END{print f+0, l+0, n+0}')"
if [ "$FIRST" -ne 0 ]; then echo "ERROR: first frame is #$FIRST, expected #00000" >&2; exit 1; fi
if [ $((LAST + 1)) -ne "$COUNT" ]; then
  echo "ERROR: frame numbering has a gap ($COUNT files, #0..#$(printf %05d "$LAST"))" >&2
  echo "       re-render the missing frames before muxing" >&2
  exit 1
fi
FRAMES_N=$COUNT

if [ -n "$DUR" ]; then
  EXPECTED=$(awk -v f="$FPS" -v d="$DUR" 'BEGIN{printf "%d", f*d + 0.5}')
  if [ "$FRAMES_N" -ne "$EXPECTED" ]; then
    echo "ERROR: $FRAMES_N frames on disk, but ${DUR}s @ ${FPS}fps needs $EXPECTED" >&2
    echo "       likely a stale tail from an older longer cut: delete frames >= #$(printf %05d "$EXPECTED") or rerun render_frames.mjs --clean" >&2
    exit 1
  fi
fi

if [ -n "$AUDIO" ] && [ -f "$AUDIO" ] && [ -n "$DUR" ]; then
  ALEN=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$AUDIO")
  DIFF=$(awk -v a="$ALEN" -v d="$DUR" 'BEGIN{r=a-d; if(r<0)r=-r; printf "%.2f", r}')
  if awk -v x="$DIFF" 'BEGIN{exit !(x > 0.35)}'; then
    echo "WARNING: audio is ${ALEN}s, timeline is ${DUR}s (diff ${DIFF}s) — captions/visuals will drift" >&2
  fi
fi

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
VOUT=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$OUT")
echo "video: $OUT  $(du -h "$OUT" | cut -f1)  ${VOUT}s  ${FRAMES_N} frames @ ${FPS}fps"

# Optional alpha preset (transparent overlay for compositing):
#   make_alpha.sh <frames-dir> <out.webm> <fps>
