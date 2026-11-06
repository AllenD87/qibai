# -*- coding: utf-8 -*-
"""本地 SSE mock vLLM：端到端验证 流式 LLM → 分句 → TTS 下行 编排。"""
import asyncio
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest

from pipecat_voice import llm, protocol
from pipecat_voice.session import RealtimeSession

# mock 流式内容（两句）
MOCK_DELTAS = ["你好。", "我在听", "呢！"]


class _Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):  # 静音
        pass

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length) or b"{}")
        assert body.get("stream") is True, "session 必须请求 stream=true"
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()
        for d in MOCK_DELTAS:
            payload = {"choices": [{"delta": {"content": d}}]}
            self.wfile.write(f"data: {json.dumps(payload, ensure_ascii=False)}\n\n".encode())
            self.wfile.flush()
            asyncio_run = None  # 纯线程：无 sleep 也要让 fake async 调度看得到顺序
            import time
            time.sleep(0.02)
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()


@pytest.fixture(scope="module")
def mock_server():
    srv = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
    port = srv.server_address[1]
    t = threading.Thread(target=srv.serve_forever, daemon=True)
    t.start()
    yield port
    srv.shutdown()


def test_stream_end_to_end(mock_server, monkeypatch):
    monkeypatch.setattr(llm, "DEFAULT_BASE", f"http://127.0.0.1:{mock_server}/v1")

    class Recorder:
        def __init__(self):
            self.events = []

        async def send(self, raw):
            self.events.append(json.loads(raw))

    rec = Recorder()
    session = RealtimeSession(asr=object(), tts_fn=lambda s: b"pcm:" + s.encode())

    async def scenario():
        await session.attach(rec.send)
        await session._run_turn("在吗")

    asyncio.run(scenario())
    types = [e["type"] for e in rec.events]
    # 关键：首句音频在流式结束前就到达（边生成边下行）
    assert types[0] == "response.created"
    assert types[-1] == "response.done"
    audio_idx = types.index("response.audio.delta")
    text_after_first = types[1:audio_idx]
    assert text_after_first == ["response.audio_transcript.delta"]  # 首句文本→首句音频
    # 两句：你好。 / 我在听呢！
    audios = [i for i, t in enumerate(types) if t == "response.audio.delta"]
    assert len(audios) == 2
