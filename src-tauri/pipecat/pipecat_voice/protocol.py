# -*- coding: utf-8 -*-
"""OpenAI Realtime 兼容事件子集（ws://127.0.0.1:8765/v1/realtime）。

只实现栖伴 B 线（Rust realtime_voice_frontend 契约层）需要的收敛面：
客户端上行：session.update / input_audio_buffer.append / response.create
服务端下行：session.updated / input_audio_buffer.speech_started|stopped /
  conversation.item.input_audio_transcription.completed /
  response.created|audio_transcript.delta|audio.delta|done / error

设计原则：事件名与 OpenAI Realtime API 对齐，B 线按官方 spec 即可对接。
"""
from __future__ import annotations

import base64
import json
from typing import Any

# 服务端事件类型
EV_SESSION_UPDATED = "session.updated"
EV_SPEECH_STARTED = "input_audio_buffer.speech_started"
EV_SPEECH_STOPPED = "input_audio_buffer.speech_stopped"
EV_TRANSCRIPTION = "conversation.item.input_audio_transcription.completed"
EV_RESPONSE_CREATED = "response.created"
EV_RESPONSE_TEXT_DELTA = "response.audio_transcript.delta"
EV_RESPONSE_AUDIO_DELTA = "response.audio.delta"
EV_RESPONSE_DONE = "response.done"
EV_ERROR = "error"

# 客户端事件类型
CE_SESSION_UPDATE = "session.update"
CE_AUDIO_APPEND = "input_audio_buffer.append"
CE_RESPONSE_CREATE = "response.create"


def _ev(t: str, payload: dict[str, Any] | None = None) -> str:
    body: dict[str, Any] = {"type": t}
    if payload:
        body.update(payload)
    return json.dumps(body, ensure_ascii=False)


def session_updated(model: str, voice: str) -> str:
    return _ev(EV_SESSION_UPDATED, {"session": {"model": model, "voice": voice}})


def speech_started() -> str:
    return _ev(EV_SPEECH_STARTED)


def speech_stopped() -> str:
    return _ev(EV_SPEECH_STOPPED)


def transcription_completed(transcript: str) -> str:
    return _ev(EV_TRANSCRIPTION, {"transcript": transcript})


def response_created() -> str:
    return _ev(EV_RESPONSE_CREATED)


def response_text_delta(delta: str) -> str:
    return _ev(EV_RESPONSE_TEXT_DELTA, {"delta": delta})


def response_audio_delta(pcm_s16_16k: bytes) -> str:
    """下行音频帧：base64 编码的 16k mono s16 PCM。"""
    return _ev(EV_RESPONSE_AUDIO_DELTA, {"delta": base64.b64encode(pcm_s16_16k).decode()})


def response_done(status: str = "completed") -> str:
    return _ev(EV_RESPONSE_DONE, {"response": {"status": status}})


def error_event(message: str) -> str:
    return _ev(EV_ERROR, {"error": {"message": message}})


def parse(raw: str) -> dict[str, Any]:
    """解析客户端事件；非法输入抛 ValueError。"""
    obj = json.loads(raw)
    if not isinstance(obj, dict) or "type" not in obj:
        raise ValueError("event missing 'type'")
    return obj


def decode_audio_append(obj: dict[str, Any]) -> bytes:
    """input_audio_buffer.append 的 audio 字段（base64）→ 原始字节。"""
    audio = obj.get("audio", "")
    return base64.b64decode(audio)
