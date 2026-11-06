#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""sherpa-onnx FunASR Paraformer-zh 中文离线转写（punc 标点模型可选）。
用法:
  python3 sherpa_stt.py --wav <f.wav> --paraformer <paraformer.onnx> \
      --tokens <tokens.txt> --vad <vad.onnx|silero_vad.onnx> [--punc <punc.onnx>]
依赖: pip install sherpa-onnx  （onnxruntime 随 wheel 捆绑；模型由 fetch_sherpa_models.sh 拉取）
"""
import argparse
import sys
import wave

import numpy as np
import sherpa_onnx


def load_wav(path):
    with wave.open(path, "rb") as wf:
        sr = wf.getframerate()
        raw = wf.readframes(wf.getnframes())
    samples = np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0
    return sr, samples


def resample(samples, src_sr, dst_sr=16000):
    """非 16k 输入做线性重采样兜底（sherpa-onnx 需要 16k）。"""
    if src_sr == dst_sr:
        return samples
    n_out = int(round(len(samples) * dst_sr / src_sr))
    x = np.linspace(0, len(samples) - 1, n_out)
    return np.interp(x, np.arange(len(samples)), samples).astype(np.float32)


def build_recognizer(args):
    """双版本兼容：
    A) 新版 sherpa-onnx（OfflineRecognizerConfig + from_config）
    B) 旧版 sherpa-onnx（from_paraformer 专用构造函数，无 from_config）
    自动探测，任一版本可用。
    """
    # ---- 方案 A：新版 ----
    if hasattr(sherpa_onnx.OfflineRecognizer, "from_config"):
        try:
            model = sherpa_onnx.OfflineModelConfig(
                paraformer=sherpa_onnx.OfflineParaformerModelConfig(model=args.paraformer),
                tokens=args.tokens,
                num_threads=4,
                provider="cpu",
                debug=False,
            )
            if args.punc:
                try:
                    model.punctuation = sherpa_onnx.OfflinePunctuationModelConfig(model=args.punc)
                except Exception as e:  # 个别版本 punctuation 字段名不同：跳过不阻断主链路
                    sys.stderr.write(f"punc 模型加载失败（跳过标点）: {e}\n")
            config = sherpa_onnx.OfflineRecognizerConfig(
                feat_config=sherpa_onnx.FeatureExtractorConfig(sampling_rate=16000),
                model_config=model,
                decoding_method="greedy_search",
            )
            return sherpa_onnx.OfflineRecognizer.from_config(config)
        except (AttributeError, TypeError) as e:
            sys.stderr.write(f"新版 API 不可用（{e}），尝试旧版 from_paraformer\n")
    # ---- 方案 B：旧版 ----
    if hasattr(sherpa_onnx.OfflineRecognizer, "from_paraformer"):
        return sherpa_onnx.OfflineRecognizer.from_paraformer(
            paraformer=args.paraformer,
            tokens=args.tokens,
            num_threads=4,
            provider="cpu",
            decoding_method="greedy_search",
            debug=False,
        )
    raise RuntimeError(
        "当前 sherpa-onnx 版本既不支持 from_config 也不支持 from_paraformer，"
        "请升级：pip3 install -U sherpa-onnx"
    )


def main():
    ap = argparse.ArgumentParser(description="sherpa-onnx paraformer 离线转写")
    ap.add_argument("--wav", required=True)
    ap.add_argument("--paraformer", required=True)
    ap.add_argument("--tokens", required=True)
    ap.add_argument("--vad", required=True)   # 完整性占位（离线整段识别不依赖 VAD）
    ap.add_argument("--punc", default="")     # 可选标点模型
    args = ap.parse_args()

    sr, samples = load_wav(args.wav)
    samples = resample(samples, sr, 16000)

    recognizer = build_recognizer(args)
    stream = recognizer.create_stream()
    stream.accept_waveform(16000, samples)
    recognizer.decode_stream(stream)
    text = stream.result.text.strip()
    if not text:
        sys.stderr.write("paraformer: 未识别到语音内容\n")
        return 1
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
