# -*- coding: utf-8 -*-
"""ws://127.0.0.1:8765/v1/realtime 服务入口。

用法：
  python3 -m pipecat_voice.server            # 默认 127.0.0.1:8765
  QIBAI_REALTIME_PORT=8765 python3 -m pipecat_voice.server

B 线 Rust 客户端按 OpenAI Realtime spec 连 /v1/realtime。
"""
from __future__ import annotations

import asyncio
import os
import sys

import websockets

from . import config
from .asr import make_asr
from .session import RealtimeSession
from .tts import make_tts, warmup_chattts

HOST = "127.0.0.1"
PORT = int(os.environ.get("QIBAI_REALTIME_PORT", "8765"))

# 启动时解析后端（auto 检测 CUDA；env 可强制覆盖）
ASR_BACKEND = config.resolve_asr_backend()
TTS_BACKEND = config.resolve_tts_backend()


async def _handler(websocket) -> None:
    path = getattr(websocket, "path", "") or ""
    if path and path != "/v1/realtime":
        await websocket.close(code=1008, reason="use /v1/realtime")
        return
    print(f"[server] client connected: {websocket.remote_address}", file=sys.stderr)
    asr = make_asr(ASR_BACKEND)
    tts_fn = make_tts(TTS_BACKEND)
    session = RealtimeSession(asr, tts_fn)

    async def send(msg: str) -> None:
        await websocket.send(msg)

    await session.attach(send)
    try:
        async for raw in websocket:
            await session.handle(raw)
    except websockets.exceptions.ConnectionClosed:
        print("[server] client disconnected", file=sys.stderr)


async def main() -> None:
    print(f"[server] asr={ASR_BACKEND} tts={TTS_BACKEND} gpu={config.gpu_available()}", file=sys.stderr)
    # 预热 ChatTTS（后台线程，不阻塞启动）：模型加载 5-15s 提前完成，首句即时推理
    if TTS_BACKEND == "chattts":
        import threading
        threading.Thread(target=warmup_chattts, daemon=True).start()
    print(f"[server] listening ws://{HOST}:{PORT}/v1/realtime", file=sys.stderr)
    async with websockets.serve(_handler, HOST, PORT, max_size=4 * 1024 * 1024):
        await asyncio.Future()  # run forever


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass
