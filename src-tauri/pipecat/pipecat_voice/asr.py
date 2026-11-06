# -*- coding: utf-8 -*-
"""ASR 引擎：sherpa-onnx paraformer-zh(int8) + silero VAD。

与任务三 scripts/sherpa_stt.py 同源同模型文件（src-tauri/models/sherpa/：
paraformer.onnx / tokens.txt / silero_vad.onnx）。差异：这里是流式喂入——
按 32ms 块上行喂给 VAD，出句即解码，供 Realtime 服务低延迟转写。

对外只暴露 feed()：喂入新 PCM，返回 (是否检测到用户起句边沿, 已出句的转写列表)。
"""
from __future__ import annotations

import os
import sys
from dataclasses import dataclass

import numpy as np
import sherpa_onnx

DEFAULT_MODEL_DIR = os.environ.get(
    "QIBAI_SHERPA_DIR",
    os.path.join(os.path.dirname(__file__), "..", "..", "models", "sherpa"),
)
# GPU 档 SenseVoice 模型位置：默认走 FunAudioLLM/SenseVoiceSmall（modelscope/hf 缓存或下载）；
# 全量离线包场景用 QIBAI_SENSEVOICE_MODEL 指本地目录（funasr.AutoModel 支持本地路径），避免联网拉取。
SENSEVOICE_MODEL = os.environ.get("QIBAI_SENSEVOICE_MODEL", "FunAudioLLM/SenseVoiceSmall")


@dataclass
class AsrFrame:
    speech_start_edge: bool  # 本次喂入是否首次检测到用户说话起句（barge-in 信号）
    transcripts: list[str]  # 本次喂入新出句的转写（已去空白）


class SherpaAsr:
    def __init__(self, model_dir: str | None = None, threads: int = 2) -> None:
        d = model_dir or DEFAULT_MODEL_DIR
        self.recognizer = sherpa_onnx.OfflineRecognizer.from_paraformer(
            paraformer=os.path.join(d, "paraformer.onnx"),
            tokens=os.path.join(d, "tokens.txt"),
            num_threads=threads,
            provider="cpu",
            debug=False,
        )
        vc = sherpa_onnx.VadModelConfig()
        vc.silero_vad.model = os.path.join(d, "silero_vad.onnx")
        vc.silero_vad.threshold = 0.5
        vc.silero_vad.min_speech_duration = 0.2
        vc.silero_vad.min_silence_duration = 0.5
        vc.sample_rate = 16000
        self.vad = sherpa_onnx.VoiceActivityDetector(vc, buffer_size_in_seconds=60)
        self._in_speech = False
        # 流式缓冲：B 线客户端按 20ms/帧(320 采样@16k)上行，VAD 需 512 采样(32ms)块——
        # 不足 512 的尾帧攒入 _buf，凑满一个块再喂 VAD，避免整帧丢弃（"听不到用户"根因）。
        self._buf: np.ndarray = np.zeros(0, dtype=np.float32)

    def feed(self, pcm_s16: bytes) -> AsrFrame:
        samples = np.frombuffer(pcm_s16, dtype=np.int16).astype(np.float32) / 32768.0
        self._buf = np.concatenate([self._buf, samples])
        block = 512  # 16k 下 32ms
        transcripts: list[str] = []
        start_edge = False
        while len(self._buf) >= block:
            chunk, self._buf = self._buf[:block], self._buf[block:]
            self.vad.accept_waveform(chunk)
            # 起句边沿：VAD 内部判为说话且本块是首次
            # （VoiceActivityDetector 不直接报边沿，用能量粗判补充）
            if not self._in_speech and float(np.abs(chunk).mean()) > 0.008:
                self._in_speech = True
                start_edge = True
            while not self.vad.empty():
                seg = self.vad.front.samples
                self.vad.pop()
                self._in_speech = False  # 出句，等待下一起句
                s = self.recognizer.create_stream()
                s.accept_waveform(16000, seg)
                self.recognizer.decode_stream(s)
                text = s.result.text.strip()
                if text:
                    transcripts.append(text)
        return AsrFrame(speech_start_edge=start_edge, transcripts=transcripts)


class SenseVoiceAsr:
    """GPU 档大中文 ASR：FunASR SenseVoiceSmall（zh/yue/en/ja/ko）。

    懒加载：import funasr 与模型加载只在工厂选择该后端时发生。
    模型经 hf-mirror.com / ModelScope 下载（见 scripts/download_sensevoice_models.sh）。
    """

    def __init__(self, device: str | None = None) -> None:
        import funasr  # type: ignore  # 懒导入
        self.model = funasr.AutoModel(
            model=SENSEVOICE_MODEL,
            device=device or ("cuda" if os.environ.get("QIBAI_FORCE_CPU") != "1" else "cpu"),
            disable_update=True,
        )
        self._in_speech = False

    def feed(self, pcm_s16: bytes) -> AsrFrame:
        samples = np.frombuffer(pcm_s16, dtype=np.int16).astype(np.float32) / 32768.0
        transcripts: list[str] = []
        start_edge = False
        # SenseVoiceSmall 为离线整句模型：用能量粗判出句（与 sherpa 档同一边沿口径）
        if float(np.abs(samples).mean()) > 0.008:
            start_edge = not self._in_speech
            self._in_speech = True
        else:
            if self._in_speech:
                self._in_speech = False
                res = self.model.generate(input=(samples * 32767.0).astype(np.int16))
                text = str(res[0]["text"]).strip()
                if text:
                    transcripts.append(text)
        return AsrFrame(speech_start_edge=start_edge, transcripts=transcripts)


def make_asr(backend: str):
    """按开关构造 ASR 后端。funasr/sensevoice 失败时回落 sherpa（CPU 保底链）。"""
    if backend in ("sensevoice", "funasr"):
        try:
            return SenseVoiceAsr()
        except Exception as e:  # GPU/模型不可用→CPU 保底
            print(f"[asr] sensevoice backend failed ({e}); fallback sherpa", file=sys.stderr)
    return SherpaAsr()
