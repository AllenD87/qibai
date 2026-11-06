#!/usr/bin/env bash
# 元气小小 v20 真机一键冒烟脚本（引导式 checklist，非破坏性）
# 用法：bash scripts/smoke_v20.sh
# 结果写入：smoke_v20_result_<YYYYmmdd_HHMMSS>.log
set -u
TS=$(date +%Y%m%d_%H%M%S)
LOG="smoke_v20_result_${TS}.log"
PASS=0; FAIL=0; SKIP=0
echo "元气小小 v20 真机冒烟 $(date)" | tee "$LOG"

# ask <步骤号> <操作> <预期> <判定标准> <问题编号>
ask() {
  local id="$1" op="$2" exp="$3" judge="$4" ref="$5"
  echo "" | tee -a "$LOG"
  echo "【步骤 $id】$op"  | tee -a "$LOG"
  echo "  预期：$exp"      | tee -a "$LOG"
  echo "  判定：$judge"    | tee -a "$LOG"
  echo "  对应：$ref"      | tee -a "$LOG"
  printf "  结果 (y=通过 / n=失败 / s=跳过): "
  read -r ans </dev/tty
  case "$ans" in
    y|Y) echo "$id  PASS" | tee -a "$LOG"; PASS=$((PASS+1));;
    n|N) echo "$id  FAIL" | tee -a "$LOG"; FAIL=$((FAIL+1));;
    *)   echo "$id  SKIP" | tee -a "$LOG"; SKIP=$((SKIP+1));;
  esac
}

# ── L0 构建 ──
ask L0.1 "在工程根执行 cd src-tauri && cargo build（或 cargo tauri dev）" \
  "编译通过无 error" "真机系统依赖装齐后 0 error" "v20 遗留#1"
ask L0.2 "确认 src-tauri/whisper-bin/whisper-server(+whisper-cli) 与 src-tauri/models/ggml-tiny.bin 在位" \
  "两资源目录存在" "voice_diag 面板不出现 STT 红卡" "P0-S"

# ── L1 出声全链路 ──
ask L1.1 "控制台手动输入发一条对话" "桌宠朗读出声" "听到语音回答" "问题1/1-R"
ask L1.2 "设置/诊断面板点「播放测试音」" "听到 880Hz 0.5s 蜂鸣；面板返回 duration_ms" "蜂鸣清晰可闻" "FE-1/问题1"
ask L1.3 "触发一次技能回复（如问天气/触发已接技能）" "技能回复口头说出声" "听到技能回答（不只是文字气泡）" "SND-1"
ask L1.4 "断开外网后重启触发朗读" "8s 内自动降级 piper/espeak 出声；日志见 [qibai] tts-chain" "断网仍有声、有降级日志" "P0-N"
ask L1.5 "触发喇叭重读/提醒类朗读（手动重读上一条）" "出声且日志只打 text_len 不打原文" "有声；stderr grep 不到朗读原文" "P0-2 隐私"

# ── L2 免提唤醒 → STT → LLM → TTS 全链路 ──
ask L2.1 "对桌宠说唤醒词" "进入 active 态；voice_diag state 翻转、rms 跳动" "唤醒词被识别" "P0-S"
ask L2.2 "唤醒后说一句话指令" "回答直接出声（免提路径）" "听到回答语音" "问题1-R/VOICE-4"
ask L2.3 "免提回答出声时观察是否双响" "只听到一个声音（Rust 播放 + 前端只演出不朗读）" "无双响；secret(保险箱)类回复不朗读" "VOICE-4"
ask L2.4 "朗读中插话（说「停」）" "朗读停止 + toast「已停止朗读（检测到你说话）」" "打断生效" "问题2"
ask L2.5 "朗读中咳嗽/背景噪音" "不误触打断" "安静继续朗读" "VOICE-1 对照"
ask L2.6 "朗读结束后观察 mic 恢复" "speaking 标记清除后 mic 可再次唤醒" "恢复安静后能再次唤醒" "VOICE-5 对照"

# ── L3 桌宠显示与交互 ──
ask L3.1 "拖动桌宠到屏幕边缘松手" "近边缘自动吸附贴边" "四边吸附生效；设置可关" "问题6"
ask L3.2 "（多屏）把桌宠拖到副屏" "副屏负坐标下不被拉回主屏原点" "无跳回主屏现象" "问题6"
ask L3.3 "发一条 3-4 行长文案触发气泡" "气泡不遮角色身体" "角色立绘完整可见" "7-P0-3"
ask L3.4 "（X11 无合成器）裸桌面拖动桌宠" "拖动中途无残影累积" "无重影" "7-P0-3"

# ── L4 系统集成 ──
ask L4.1 "托盘菜单：显示主窗/隐藏主窗/退出" "三项均可用；退出前 netsocial 优雅断开" "托盘行为正常" "7-P1-6"
ask L4.2 "控制台窗点关闭按钮" "窗关闭成功（withGlobalTauri=false 适配后）" "按钮有效" "close_console"
ask L4.3 "长对话（>50 条）后查看历史" "历史完整不丢段；异常关闭重启后不静默清空" "无历史丢失" "问题4"
ask L4.4 "删除 whisper-bin/models 后启动" "诊断面板红色 STT 自愈卡（期望路径+下载链接）" "红卡出现且链接可点" "P0-S"
ask L4.5 "聊天页切到别的 tab 再切回 + 连发两条消息" "历史不回退、不丢段；第二条消息发送时第一条被锁（不并发）" "epoch 门/发送锁生效" "问题4"

# ── L5 三平台/钥匙串（按真机系统选做） ──
ask L5.1 "保险箱：设主密码→锁定/解锁/复制/导出→重启" "钥匙串自动解锁；剪贴板写入后定时清除" "vault 全流程正常" "隐私"
ask L5.2 "release 构建后触发 execute_command 类技能" "危险命令审批弹窗出现（不直接执行）" "OPEN_ALL 已收紧" "7-P0-1"

echo "" | tee -a "$LOG"
echo "==== 汇总：PASS=$PASS  FAIL=$FAIL  SKIP=$SKIP  （日志：$LOG）" | tee -a "$LOG"
echo "FAIL 项请对照报告 §3 问题编号回链排查。" | tee -a "$LOG"
