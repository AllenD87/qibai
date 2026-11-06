#!/usr/bin/env bash
# ============================================================
# 元气小小 · Piper 中文女声模型下载脚本（可选增强，非必需）
# 模型：zh_CN-huayan-medium（花颜，VITS/ONNX，~63MB，22050Hz，女声）
# 来源：ModelScope（国内可达，比 HuggingFace 快）
# 用法：bash scripts/piper_download.sh
# 之后安装运行时：pip install piper-tts
# 音色链：edge-tts（晓晓，在线）→ Piper 花颜（离线）→ espeak（兜底）
# ============================================================
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="$ROOT/assets/tts/piper"
mkdir -p "$DIR"
echo "[1/2] 下载模型 model.onnx ..."
curl -L --fail --progress-bar -o "$DIR/zh_CN-huayan-medium.onnx" \
  "https://modelscope.cn/models/Trelis/piper-zh-cn-huayan-medium/resolve/master/model.onnx"
echo "[2/2] 下载配置 model.onnx.json ..."
curl -L --fail --progress-bar -o "$DIR/zh_CN-huayan-medium.onnx.json" \
  "https://modelscope.cn/api/v1/models/Trelis/piper-zh-cn-huayan-medium/repo?Revision=master&FilePath=model.onnx.json"
echo "完成：$DIR"
ls -lh "$DIR"
echo "下一步：pip install -i https://mirrors.aliyun.com/pypi/simple/ --trusted-host mirrors.aliyun.com piper-tts（阿里云镜像，一次即可），重启元气小小自动启用离线女声。"
