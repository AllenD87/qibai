#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""qibai STT runner：sherpa-onnx paraformer-zh(int8) + silero VAD + ct-punc。

被 qibai-core 通过子进程调用（与 whisper-cli 同构）。输入一个 16k 单声道 wav 路径，
stdout 输出带标点的转写文本（单行，末尾换行）。出错时 stderr 打印原因并不带 0 退出。

用法：
  python3 sherpa_stt.py --wav FILE --paraformer ONNX --tokens TXT --vad ONNX --punc ONNX

设计：模型路径全部由 Rust 侧解析后显式传入，本脚本不做路径猜测，便于单测与排障。
依赖：pip install sherpa-onnx（onnxruntime 已随 wheel 捆绑，无需系统 onnxruntime）。
"""
import argparse
import sys
import wave

import numpy as np
import sherpa_onnx


def read_wav16k(path: str) -> np.ndarray:
    with wave.open(path) as w:
        if w.getnchannels() != 1 or w.getsampwidth() != 2:
            raise SystemExit(f"expect mono s16 wav, got ch={w.getnchannels()} sw={w.getsampwidth()}")
        sr = w.getframerate()
        data = w.readframes(w.getnframes())
    samples = np.frombuffer(data, dtype=np.int16).astype(np.float32) / 32768.0
    return sr, samples


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--wav", required=True)
    ap.add_argument("--paraformer", required=True)
    ap.add_argument("--tokens", required=True)
    ap.add_argument("--vad", required=True)
    ap.add_argument("--punc", required=True)
    ap.add_argument("--threads", type=int, default=2)
    args = ap.parse_args()

    sr, samples = read_wav16k(args.wav)
    if sr != 16000:
        # 生产 listener 已重采样到 16k；这里兜底重采样（线性）。
        ratio = 16000 / sr
        n = int(len(samples) * ratio)
        samples = np.interp(np.linspace(0, len(samples) - 1, n), np.arange(len(samples)), samples).astype(np.float32)

    recog = sherpa_onnx.OfflineRecognizer.from_paraformer(
        paraformer=args.paraformer, tokens=args.tokens,
        num_threads=args.threads, provider="cpu", debug=False,
    )

    vc = sherpa_onnx.VadModelConfig()
    vc.silero_vad.model = args.vad
    vc.silero_vad.threshold = 0.5
    vc.silero_vad.min_speech_duration = 0.2
    vc.silero_vad.min_silence_duration = 0.5
    vc.sample_rate = 16000
    vad = sherpa_onnx.VoiceActivityDetector(vc, buffer_size_in_seconds=60)
    # silero VAD 需按小块（16k 下 512≈32ms）喂入；整段喂入会失效。
    block = 512
    for i in range(0, len(samples) - block + 1, block):
        vad.accept_waveform(samples[i:i + block])
    vad.flush()

    parts = []
    while not vad.empty():
        seg = vad.front.samples
        vad.pop()
        s = recog.create_stream()
        s.accept_waveform(16000, seg)
        recog.decode_stream(s)
        t = s.result.text.strip()
        if t:
            parts.append(t)
    raw = "".join(parts)

    try:
        pm = sherpa_onnx.OfflinePunctuationModelConfig(
            ct_transformer=args.punc, num_threads=args.threads, provider="cpu", debug=False)
        punc = sherpa_onnx.OfflinePunctuation(
            config=sherpa_onnx.OfflinePunctuationConfig(model=pm))
        out = punc.add_punctuation(raw)
    except Exception as e:  # 标点失败不阻断转写
        print(f"[warn] punc failed: {e}", file=sys.stderr)
        out = raw

    sys.stdout.write(out.strip() + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
