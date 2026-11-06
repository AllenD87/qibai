# -*- coding: utf-8 -*-
"""后端开关解析：ASR_BACKEND / TTS_BACKEND / GPU 检测与 env 强制覆盖。

设计：
- ASR_BACKEND = auto | sherpa | sensevoice | funasr
  auto → 启动时 torch.cuda.is_available()：GPU 可用优先 sensevoice（大中文模型），
         不可用回落 sherpa int8（CPU 保底链）。
- TTS_BACKEND = auto | null | edge | chattts | qwen3
  auto → GPU 可用优先 chattts，否则 edge（网络流式）；edge 不可用时兜底 null。
- 环境变量可强制覆盖：ASR_BACKEND / TTS_BACKEND / QIBAI_FORCE_CPU=1。
"""
from __future__ import annotations

import os

ASR_OPTIONS = ("auto", "sherpa", "sensevoice", "funasr")
TTS_OPTIONS = ("auto", "null", "edge", "chattts", "qwen3")


def gpu_available() -> bool:
    """torch.cuda.is_available() 的安全包装；torch 未装视为无 GPU。"""
    if os.environ.get("QIBAI_FORCE_CPU", "") == "1":
        return False
    try:
        import torch  # type: ignore
        return bool(torch.cuda.is_available())
    except Exception:
        return False


def resolve_asr_backend() -> str:
    raw = os.environ.get("ASR_BACKEND", "auto").strip().lower()
    if raw not in ASR_OPTIONS:
        raise ValueError(f"unknown ASR_BACKEND={raw!r}, expect one of {ASR_OPTIONS}")
    if raw != "auto":
        return raw
    return "sensevoice" if gpu_available() else "sherpa"


def resolve_tts_backend() -> str:
    raw = os.environ.get("TTS_BACKEND", "auto").strip().lower()
    if raw not in TTS_OPTIONS:
        raise ValueError(f"unknown TTS_BACKEND={raw!r}, expect one of {TTS_OPTIONS}")
    if raw != "auto":
        return raw
    return "chattts" if gpu_available() else "edge"
