#!/usr/bin/env bash
# 元气小小 TTS 语音引擎一键安装（三选一即可发声，重复运行无害）
set -e
echo "== 元气小小语音引擎检查 =="
for b in edge-tts piper espeak-ng espeak spd-say paplay aplay; do
  if command -v "$b" >/dev/null 2>&1; then echo "  有: $b"; else echo "  无: $b"; fi
done
echo
echo "== 方案A（推荐·自然女声·在线）：pip install edge-tts =="
if command -v pip3 >/dev/null 2>&1; then
  read -r -p "现在安装 edge-tts？[y/N] " ans
  if [ "$ans" = "y" ] || [ "$ans" = "Y" ]; then
    pip3 install --user edge-tts && echo "edge-tts 已安装，元气小小自动使用「晓晓」女声朗读。"
  fi
else
  echo "  未找到 pip3，请先安装 Python3 + pip 再执行本脚本。"
fi
echo
echo "== 方案B（离线·神经网络女声）：bash scripts/piper_download.sh =="
echo "== 方案C（兜底·机械音）：sudo apt install espeak-ng =="
echo
echo "装好后重启元气小小，问它一句「你好」即可验证朗读。"
