#!/usr/bin/env bash
# ============================================================
# 元气小小 · 音色一键准备（可选增强，两条都装效果最好）
# ① edge-tts  —— 微软晓晓（在线，最自然女声，免 key，隐私=只发当前朗读片段）
# ② piper-tts —— 离线女声引擎（模型已内置 assets/tts/piper，无需再下载）
# 用法：bash scripts/tts_setup.sh   （需要联网装一次）
# ============================================================
set -e
echo "[1/2] 安装 edge-tts（在线晓晓女声，阿里云镜像）..."
pip install --user -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com edge-tts
echo "[2/2] 安装 piper-tts（离线花颜女声，模型已在包内，阿里云镜像）..."
pip install --user -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com piper-tts
echo
echo "完成！重启元气小小即可。
音色链：edge-tts 晓晓（在线）→ Piper 花颜（离线）→ espeak 女声变体（兜底）。
什么都不装也能出声（espeak 女声变体），装了则逐级变好听。"
