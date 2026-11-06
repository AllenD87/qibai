#!/usr/bin/env bash
# 元气小小 v21 版本核对脚本 —— 判断真机源码是否已应用最新修复
# 用法：在工程根执行  bash scripts/check_version.sh
# 原理：v20 起的关键修复均在源码留有关键锚点（函数名/日志前缀/分支），
#       一键 grep 断言，逐项 PASS/FAIL，缺哪项即缺对应版本功能。
# 无破坏性操作，只读检查。
set -u
PASS=0; FAIL=0
echo "元气小小版本核对 $(date)"
echo "核对范围：真机工程根源码是否为 v21 最新（含 v20 P0 兜底 / fix33 / fix34 / SND-1）"
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
  "listener.rs 免提路径 Rust 朗读兜底调用（v20 P0 真根因修复）" \
  "无 Rust 兜底=真机仍是 v19 或更早；请解压 v21 增量包覆盖后再验"
check A2 src-tauri/src/listener.rs "speak_and_emit_pet_say" \
  "listener.rs 统一发音 helper（v21 SND-1：技能/意图/对话五处统一）" \
  "缺统一 helper=未应用 v21 增量包；技能回复（如「打开设置」）将无声"
check A3 src-tauri/src/listener.rs "rust_played" \
  "listener.rs rust_played 防双响标志（v20/v21）" \
  "无防双响标志=旧版；真机可能出现 Rust 与前端双响或前端缺口无声"

echo ""
echo "── B 组：手动输入路径与出声锚点日志（v20 fix31 + v21）──"
check B1 src-tauri/src/main.rs "perform_speech" \
  "main.rs Rust 朗读兜底函数与调用（手动输入对话出声）" \
  "缺兜底=真机为 v19 或更早；控制台手动输入对话将无声"
check B2 src-tauri/src/main.rs "qibai-voice" \
  "main.rs [qibai-voice] 出声链路锚点日志（v21）" \
  "缺锚点日志=未应用 v21；真机无声时将无法按锚点对照表定位断链层"

echo ""
echo "── C 组：reasoning 模型降级（fix34）──"
check C1 qibai-core/src/llm_client.rs "reasoning_content" \
  "llm_client.rs 流式 reasoning_content 识别分支（fix34）" \
  "缺 fix34=推理模型对话将双重耗时（等完整流式 30-60s 再降级）"
check C2 qibai-core/src/llm_client.rs "跳过流式直接降级非流式" \
  "llm_client.rs fix34 锚点日志「跳过流式直接降级非流式」" \
  "缺锚点日志=未应用 fix34；真机推理模型对话首字延迟将达 30-60s"

echo ""
echo "── D 组：截断 JSON 容错（fix33）──"
check D1 qibai-core/src/persona.rs "extract_json_field_value" \
  "persona.rs 截断 JSON 字段提取函数（fix33）" \
  "缺 fix33=LLM 缺右花括号时会把 JSON 原文当回复（前端显示残缺 JSON）"
check D2 qibai-core/src/persona.rs 'format!("{json}}}")' \
  "persona.rs 补右花括号重试（fix33 第二级容错）" \
  "缺补 } 重试=未应用 fix33；截断 JSON 回复解析将回退"

echo ""
echo "==== 汇总：PASS=$PASS  FAIL=$FAIL ===="
if [ "$FAIL" -eq 0 ]; then
  echo "结论：真机源码为 v21 最新（v20 P0 兜底 + SND-1 + fix33 + fix34 全部在位）。"
  echo "若仍无声：进入无声诊断流程——① 跑 scripts/smoke_v20.sh 出声类步骤；"
  echo "② 终端 grep '[qibai-voice]' 按《语音链路日志锚点对照表》定位断链层；"
  echo "③ 对照表判定为播放层问题则执行播放层深度排查（见交付说明 A.4）。"
else
  echo "结论：真机源码缺少 $FAIL 项修复——请将 v21 增量包完整解压到工程根覆盖后再验证。"
  echo "注意：解压覆盖后需重新编译：cd src-tauri && cargo build --release（或 cargo tauri dev）。"
fi
exit 0
