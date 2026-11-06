# -*- coding: utf-8 -*-
"""会话状态机单测：barge-in 取消在途 + 一轮 response 事件序列（不依赖真模型）。"""
from __future__ import annotations

import asyncio
import json

import pytest

from pipecat_voice import llm, protocol, tts
from pipecat_voice.session import RealtimeSession


class FakeAsr:
    """可脚本化帧：speech_start_edge / transcripts。"""
    def __init__(self, frame):
        self._frame = frame

    def feed(self, pcm):
        return self._frame


class Recorder:
    def __init__(self):
        self.events: list[str] = []

    async def send(self, msg: str):
        self.events.append(msg)


def run(coro):
    return asyncio.run(coro)


def test_turn_emits_full_event_sequence(monkeypatch):
    monkeypatch.setattr(llm, "chat", lambda sys, u: "你好呀")
    rec = Recorder()
    session = RealtimeSession(FakeAsr(None))

    async def scenario():
        await session.attach(rec.send)
        await session._run_turn("在吗")  # 直接跑真实 turn 逻辑（同 loop）

    run(scenario())
    types = [json.loads(e)["type"] for e in rec.events]
    assert types == [
        "response.created",
        "response.audio_transcript.delta",
        "response.audio.delta",
        "response.done",
    ]


def test_barge_in_cancels_in_flight_turn(monkeypatch):
    import asyncio as aio
    from pipecat_voice.asr import AsrFrame

    monkeypatch.setattr(llm, "chat", lambda s, u: None)
    rec = Recorder()
    session = RealtimeSession(FakeAsr(None))

    class EdgeAsr:
        def feed(self, pcm):
            return AsrFrame(speech_start_edge=True, transcripts=[])

    async def scenario():
        await session.attach(rec.send)
        # 手动制造在途 busy 任务
        session._busy = True
        session._bot_task = aio.create_task(aio.sleep(10))
        session.asr = EdgeAsr()
        await session._on_audio(b"\x00\x00" * 512)

    run(scenario())
    types = [json.loads(e)["type"] for e in rec.events]
    assert "input_audio_buffer.speech_started" in types
    assert session._bot_task.cancelled()


def test_error_event_on_unknown_type():
    rec = Recorder()
    session = RealtimeSession(FakeAsr(None))
    run(session.attach(rec.send))
    run(session.handle(json.dumps({"type": "bogus.event"})))
    assert json.loads(rec.events[-1])["type"] == "error"
