# -*- coding: utf-8 -*-
"""后端开关解析单测（不依赖真实模型/CUDA）。"""
from __future__ import annotations

import importlib

import pytest

from pipecat_voice import config


def reload_config(monkeypatch, env):
    for k in ("ASR_BACKEND", "TTS_BACKEND", "QIBAI_FORCE_CPU"):
        monkeypatch.delenv(k, raising=False)
    for k, v in env.items():
        monkeypatch.setenv(k, v)
    importlib.reload(config)
    return config


def test_auto_picks_gpu_backends_when_cuda(monkeypatch):
    c = reload_config(monkeypatch, {})
    monkeypatch.setattr(c, "gpu_available", lambda: True)
    assert c.resolve_asr_backend() == "sensevoice"
    assert c.resolve_tts_backend() == "chattts"


def test_auto_falls_back_when_no_gpu(monkeypatch):
    c = reload_config(monkeypatch, {})
    monkeypatch.setattr(c, "gpu_available", lambda: False)
    assert c.resolve_asr_backend() == "sherpa"
    assert c.resolve_tts_backend() == "edge"


def test_env_force_override(monkeypatch):
    c = reload_config(monkeypatch, {"ASR_BACKEND": "sherpa", "TTS_BACKEND": "null"})
    monkeypatch.setattr(c, "gpu_available", lambda: True)
    assert c.resolve_asr_backend() == "sherpa"
    assert c.resolve_tts_backend() == "null"


def test_invalid_backend_rejected(monkeypatch):
    c = reload_config(monkeypatch, {"ASR_BACKEND": "nope"})
    with pytest.raises(ValueError):
        c.resolve_asr_backend()
    c2 = reload_config(monkeypatch, {"TTS_BACKEND": "nope"})
    with pytest.raises(ValueError):
        c2.resolve_tts_backend()


def test_force_cpu_env(monkeypatch):
    c = reload_config(monkeypatch, {"QIBAI_FORCE_CPU": "1"})
    assert c.gpu_available() is False
