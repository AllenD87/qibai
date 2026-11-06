#!/usr/bin/env bash
# ============================================================
# ⚠️ 已停用（用户改向 2026-10-27：不做真 3D/GLB，改 3D 国漫 CG
#    风格立绘主展示）——保留备查，勿再使用。
# ============================================================
# gen_3d_tripo.sh — 真机：Tripo image-to-3D API（自带 biped mixamo 骨架）
#
# ⚠️ 真机执行。需要 TRIPO_API_KEY（https://platform.tripo3d.ai 申请）。
# 成本注记：Tripo 文生/图生 3D 约 $5/次生成；auto-rig/retarget 另计。
# 免费额度：新账号送少量 credits（以平台当前政策为准）。
# 数据外发：上传的人设图会发到 Tripo 云端（国内服务）；本地优先隐私场景改用 gen_3d_hunyuan.sh。
#
# 用法：
#   TRIPO_API_KEY=xxx PERSONA=huopo bash scripts/gen_3d_tripo.sh
#   TRIPO_API_KEY=xxx PERSONA=all   bash scripts/gen_3d_tripo.sh
# 产物：src/assets/3d/<persona>.glb（带 mixamo 骨架，A 组可直接播 idle）

set -euo pipefail

QIBAI_ROOT="${QIBAI_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$QIBAI_ROOT"

: "${TRIPO_API_KEY:?需要环境变量 TRIPO_API_KEY}"
API="${TRIPO_API:-https://api.tripo3d.ai/v2/openapi}"
PERSONA="${PERSONA:-huopo}"
OUT_DIR="$QIBAI_ROOT/src/assets/3d"
ALL_PERSONAS="gudian dushi wenrou huopo reqing"

log()  { echo "[tripo] $*"; }
fail() { echo "[tripo][ERROR] $*" >&2; exit 1; }

command -v curl >/dev/null || fail "需要 curl"
command -v python3 >/dev/null || fail "需要 python3（解析 JSON）"

run_one() {
  local p="$1"
  local img="$QIBAI_ROOT/src/assets/ai_char/${p}_raw.png"
  [[ -f "$img" ]] || { log "跳过 $p（无输入图 $img）"; return 1; }
  mkdir -p "$OUT_DIR"
  local glb_out="$OUT_DIR/${p}.glb"
  [[ -s "$glb_out" ]] && { log "跳过 $p（已存在 $glb_out）"; return 0; }

  # 1) 上传图片
  log "[$p] 上传 $img ..."
  local upload_resp
  upload_resp=$(curl -s -X POST "$API/upload" \
    -H "Authorization: Bearer $TRIPO_API_KEY" \
    -F "file=@$img;type=image/png")
  local file_token
  file_token=$(echo "$upload_resp" | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['file_token'])")
  log "[$p] file_token=$file_token"

  # 2) 提交 image-to-3D 任务（要求绑骨 biped + mixamo 规范）
  log "[$p] 提交 image-to-3D 任务 ..."
  local task_resp
  task_resp=$(curl -s -X POST "$API/task" \
    -H "Authorization: Bearer $TRIPO_API_KEY" \
    -H "Content-Type: application/json" \
    -d "{\"type\":\"image_to_model\",\"file\":{\"type\":\"png\",\"file_token\":\"$file_token\"}}")
  local task_id
  task_id=$(echo "$task_resp" | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['task_id'])")
  log "[$p] task_id=$task_id（生成约 1-3 分钟）"

  # 3) 轮询
  local status="running"
  for i in $(seq 1 60); do
    sleep 5
    local poll
    poll=$(curl -s "$API/task/$task_id" -H "Authorization: Bearer $TRIPO_API_KEY")
    status=$(echo "$poll" | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['status'])")
    log "[$p] 轮询 #$i: status=$status"
    [[ "$status" == "success" ]] && break
    [[ "$status" == "failed" ]] && { echo "$poll" >&2; fail "[$p] Tripo 任务失败"; }
  done
  [[ "$status" == "success" ]] || fail "[$p] 轮询超时"

  # 4) 下载 GLB
  log "[$p] 下载 GLB ..."
  local model_url
  model_url=$(curl -s "$API/task/$task_id" -H "Authorization: Bearer $TRIPO_API_KEY" \
    | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['output']['pbr_model'])")
  curl -sL -o "$glb_out" "$model_url"
  log "[$p] 完成 -> $glb_out ($(du -h "$glb_out" | cut -f1))"
}

if [[ "$PERSONA" == "all" ]]; then
  for p in $ALL_PERSONAS; do run_one "$p"; done
else
  run_one "$PERSONA"
fi

log "全部完成。提示：GLB 若面数偏高，真机可跑 gltf-transform simplify 后再交 A 组。"
