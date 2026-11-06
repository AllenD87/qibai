# -*- coding: utf-8 -*-
"""Realtime 协议事件序列化/解析单测。"""
from __future__ import annotations

import base64
import json

from pipecat_voice import protocol


def test_session_updated_shape():
    msg = json.loads(protocol.session_updated("m", "v"))
    assert msg["type"] == "session.updated"
    assert msg["session"]["model"] == "m"


def test_transcription_event_roundtrip():
    msg = json.loads(protocol.transcription_completed("今天天气"))
    assert msg["type"] == "conversation.item.input_audio_transcription.completed"
    assert msg["transcript"] == "今天天气"


def test_audio_delta_base64():
    pcm = b"\x01\x02\x03"
    msg = json.loads(protocol.response_audio_delta(pcm))
    assert msg["type"] == "response.audio.delta"
    assert base64.b64decode(msg["delta"]) == pcm


def test_parse_rejects_garbage():
    import pytest
    with pytest.raises(ValueError):
        protocol.parse("not json")
    with pytest.raises(ValueError):
        protocol.parse('{"foo": 1}')


def test_decode_audio_append():
    obj = {"type": "input_audio_buffer.append",
           "audio": base64.b64encode(b"\xaa\xbb").decode()}
    assert protocol.decode_audio_append(obj) == b"\xaa\xbb"
