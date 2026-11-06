#!/usr/bin/env bash
# 元气小小 v22 合流核对脚本（正式工具）—— 一键判断真机源码是否已合流最新修复
# 用法：工程根执行  bash scripts/check_qibai_version.sh
# 背景：用户实证「解析/渲染已修复但无声、日志无任何 Rust 兜底锚点」→ 触发层未走到，
#       根因判定为真机版本未合流。本脚本以关键锚点断言版本合流完整性，PASS/FAIL 逐项输出。
# 覆盖版本：v20 P0 免提兜底 / v21 SND-1 统一 helper / fix33 截断 JSON / fix34 reasoning 降级
#           / fix29 edge-tts 超时 / v20 播放状态中枢。只读检查，无破坏性操作。
set -u
PASS=0; FAIL=0
echo "元气小小 v22 合流核对 $(date)"
echo "核对范围：真机工程根源码是否合流 v20 P0 兜底 / SND-1 / fix33 / fix34 / fix29 / 播放状态中枢"
echo ""

# check <编号> <文件> <锚点串> <说明> <缺失提示>
check() {
  local id="$1" file="$2" anchor="$3" desc="$4" hint="$5"
  if grep -qF "$anchor" "$file" 2>/dev/null; then
    echo "[PASS] $id  $desc"
    PASS=$((PASS+1))
  else
    echo "[FAIL] $id  $desc"
    echo "        缺失提示：$hint"
    FAIL=$((FAIL+1))
  fi
}

echo "── A 组：免提/技能回复 Rust 朗读兜底（v20 P0 + v21 SND-1）──"
check A1 src-tauri/src/listener.rs "super::perform_speech" \
  "listener.rs 免提路径 Rust 朗读兜底（v20 P0 真根因）" \
  "无=真机为 v19 或更早；解压 v22 增量包覆盖后重新 cargo build"
check A2 src-tauri/src/listener.rs "speak_and_emit_pet_say" \
  "listener.rs 统一发音 helper（v21 SND-1：技能/意图/对话五处统一）" \
  "缺=未应用 v21；技能回复（如「打开设置」）将无声"
check A3 src-tauri/src/listener.rs "rust_played" \
  "listener.rs rust_played 防双响标志（v20/v21）" \
  "缺=旧版；真机可能双响或前端缺口无声"

echo ""
echo "── B 组：手动输入路径与出声锚点日志 ──"
check B1 src-tauri/src/main.rs "perform_speech" \
  "main.rs Rust 朗读兜底函数与调用（手动输入对话出声）" \
  "缺=真机为 v19 或更早；控制台手动输入对话将无声"
check B2 src-tauri/src/main.rs "qibai-voice" \
  "main.rs [qibai-voice] 出声链路锚点日志前缀（v21）" \
  "缺=未应用 v21；真机无声时无法按锚点对照表定位断链层"

echo ""
echo "── C 组：reasoning 模型降级（fix34）──"
check C1 qibai-core/src/llm_client.rs "reasoning_content" \
  "llm_client.rs 流式 reasoning_content 识别分支（fix34）" \
  "缺 fix34=推理模型对话双重耗时（等完整流式 30-60s 再降级）"
check C2 qibai-core/src/llm_client.rs "跳过流式直接降级非流式" \
  "llm_client.rs fix34 锚点日志「跳过流式直接降级非流式」" \
  "缺锚点日志=未应用 fix34；真机推理模型首字延迟将达 30-60s"

echo ""
echo "── D 组：截断 JSON 容错（fix33）──"
check D1 qibai-core/src/persona.rs "extract_json_field_value" \
  "persona.rs 截断 JSON 字段提取函数（fix33）" \
  "缺 fix33=LLM 缺右花括号时把 JSON 原文当回复（前端显示残缺 JSON）"
check D2 qibai-core/src/persona.rs 'format!("{json}}}")' \
  "persona.rs 补右花括号重试（fix33 第二级容错）" \
  "缺补 } 重试=未应用 fix33；截断 JSON 回复解析将回退"

echo ""
echo "── T 组：TTS 决策链（fix29 + v20 播放中枢）──"
check T1 qibai-core/src/tts.rs "run_with_timeout" \
  "tts.rs edge-tts 超时 kill（fix29：8s 超时防断网阻塞）" \
  "缺 fix29=断网时朗读全链路静默挂死"
check T2 qibai-core/src/tts.rs "PlaybackStatus" \
  "tts.rs 播放状态中枢（v20：playing/done/failed 事件）" \
  "缺=未应用 v20；播放层结果无法归因"

echo ""
echo "==== 汇总：PASS=$PASS  FAIL=$FAIL ===="
if [ "$FAIL" -eq 0 ]; then
  echo "结论：真机源码已合流 v22 最新（v20 P0 兜底 + SND-1 + fix33 + fix34 + fix29 + 播放中枢全部在位）。"
  echo "若仍无声：进入无声诊断——① bash scripts/smoke_v20.sh 出声类步骤；"
  echo "② grep '[qibai-voice]' 按《语音链路日志锚点对照表》定位断链层；"
  echo "③ 判定播放层则按锚点表末节「播放层深度排查预案」执行。"
else
  echo "结论：真机源码缺失 $FAIL 项修复——请将 v22 增量包完整解压覆盖工程根，"
  echo "      并重新编译：cd src-tauri && cargo build --release（或 cargo tauri dev），再复跑本脚本。"
fi
exit 0
