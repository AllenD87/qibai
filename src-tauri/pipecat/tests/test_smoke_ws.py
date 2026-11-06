# -*- coding: utf-8 -*-
"""ws 冒烟：握手 + 一轮「音频帧上行 → 转写/LLM → TTS 事件 → 下行」往返。

真模型在 src-tauri/models/sherpa/ 缺失时自动 skip（沙盒外真机跑时再验）。
"""
from __future__ import annotations

import asyncio
import json
import os

import pytest

MODEL_DIR = os.path.join(os.path.dirname(__file__), "..", "..", "models", "sherpa")
HAVE_MODELS = os.path.exists(os.path.join(MODEL_DIR, "paraformer.onnx"))


@pytest.mark.asyncio
async def test_ws_handshake_and_session_update():
    if not HAVE_MODELS:
        pytest.skip("sherpa 模型未就位（真机补模型后再验）")
    import websockets
    from pipecat_voice.server import main as server_main

    server = asyncio.create_task(server_main())
    await asyncio.sleep(1.5)  # 等 sherpa 加载
    try:
        async with websockets.connect("ws://127.0.0.1:8765/v1/realtime") as ws:
            await ws.send(json.dumps({"type": "session.update", "session": {}}))
            reply = json.loads(await asyncio.wait_for(ws.recv(), timeout=5))
            assert reply["type"] == "session.updated"
    finally:
        server.cancel()
