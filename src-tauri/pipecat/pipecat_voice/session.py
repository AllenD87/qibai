# -*- coding: utf-8 -*-
"""Realtime 会话状态机：barge-in / 在途取消 / 一轮对话编排。

借鉴 Pipecat InterruptionFrame 语义（docs.pipecat.ai/pipecat/fundamentals/interruptions）：
- VAD 检测到用户起句边沿（speech_started）→ 若 Bot 正在响应（LLM/TTS 在途），
  立即取消在途任务，上下文只保留「实际已说出」部分；
- 用户新的转写出句 → 开启新一轮 response；
- 所有下行事件经 ws 推给 B 线 Rust 客户端。
"""
from __future__ import annotations

import asyncio
import difflib
import sys
import time

from . import llm, protocol, tts
from .tts import split_sentences

SYSTEM_PROMPT = "你是元气小小，温柔的桌宠，简短自然地回话。"

# 回声防护：Bot 说话中（_busy）及说完后的冷却窗内，ASR 转写不触发新回合。
# 解决"自说自话"环路：TTS 播放被麦克风/ASR 拾取 → 触发回复 → 再被拾取 → 无限循环。
# 冷却窗覆盖最后一句的播放尾巴；打断能力（speech_start_edge）不受影响。
COOLDOWN_AFTER_TTS = 4.0  # 秒（v2 的 2.0 对重度回采不够，加大到 4s）
# TTS 输出格式：16kHz mono int16（tts.py SAMPLE_RATE=16000，s16le -ac 1）
_PCM_RATE = 16000
_PCM_BYTES_PER_SAMPLE = 2
_PLAYBACK_TAIL = 1.0  # 播放结束后的额外残响缓冲（秒）


def _is_echo(transcript: str, recent_tts: str) -> bool:
    """转写文本与最近朗读文本高度重叠 → 判定为 TTS 残响回声，丢弃。
    镜像客户端 fix38 的文本层兜底（服务端侧补上这一道）。"""
    a = "".join(transcript.split())
    b = "".join(recent_tts.split())
    if not a or not b:
        return False
    if len(a) < 3 or len(b) < 3:
        return False
    if a in b or b in a:
        return True
    return difflib.SequenceMatcher(None, a, b).ratio() > 0.6


class RealtimeSession:
    def __init__(self, asr, tts_fn=None) -> None:
        self.asr = asr
        self.tts_fn = tts_fn or tts.null_tts_pcm
        self._send = None  # 由 server 注入：async def send(json_str)
        self._bot_task: asyncio.Task | None = None  # 当前在途的 response 任务
        self._busy = False  # Bot 正在响应
        self._buf = ""  # 流式 LLM 累积缓冲（未切完的句子）
        self._last_audio_sent_at = 0.0  # 自己最后一次下发 TTS 音频的时刻
        self._cooldown_until = 0.0  # 回声冷却截止时刻（按发送时刻+实际播放时长+缓冲）
        self._recent_tts_text = ""  # 最近朗读文本（回声文本层过滤用，保留约 300 字）

    async def attach(self, send) -> None:
        self._send = send

    async def handle(self, raw: str) -> None:
        obj = protocol.parse(raw)
        etype = obj["type"]
        if etype == protocol.CE_SESSION_UPDATE:
            await self._send(protocol.session_updated(model="qibai-local", voice="xiaoxiao"))
        elif etype == protocol.CE_AUDIO_APPEND:
            await self._on_audio(protocol.decode_audio_append(obj))
        elif etype == protocol.CE_RESPONSE_CREATE:
            await self._start_turn("你好")
        else:
            await self._send(protocol.error_event(f"unknown event: {etype}"))

    async def _on_audio(self, pcm: bytes) -> None:
        frame = self.asr.feed(pcm)
        now = time.time()
        # barge-in：用户起句边沿且 Bot 在说话 → 打断在途回合。
        # 只取消不开新回合：打断瞬间的转写多半混着自家 TTS 回声，
        # 立即开新回合会复现"自说自话"循环；用户后续语音（冷却窗外）自然触发。
        if frame.speech_start_edge and self._busy:
            print("[session] barge-in: cancel in-flight bot turn", file=sys.stderr)
            await self._send(protocol.speech_started())
            self._cancel_bot()
            return
        # 回声防护：Bot 说话中（_busy）或冷却截止前（发送时刻+播放时长+缓冲）→ 不触发新回合。
        if self._busy or now < self._cooldown_until:
            return
        for text in frame.transcripts:
            # 文本层回声过滤：残响转写与最近朗读内容高度重叠 → 丢弃（自说自话兜底）。
            if _is_echo(text, self._recent_tts_text):
                print(
                    f"[session] echo-discard: {text!r} ~= {self._recent_tts_text!r}",
                    file=sys.stderr,
                )
                continue
            print(f"[session] transcript: {text}", file=sys.stderr)
            await self._send(protocol.transcription_completed(text))
            await self._start_turn(text)

    def _cancel_bot(self) -> None:
        if self._bot_task and not self._bot_task.done():
            self._bot_task.cancel()
        self._busy = False

    async def _start_turn(self, user_text: str) -> None:
        """开启一轮 response：LLM → 文本增量 → TTS 音频增量 → done。"""
        # 新一轮用户发言到来，若上一轮还在跑则先取消（新话覆盖旧话）
        self._cancel_bot()
        self._busy = True
        self._bot_task = asyncio.create_task(self._run_turn(user_text))

    async def _run_turn(self, user_text: str) -> None:
        """边生成边下行：LLM 流式 → 按句切分 → 首句 TTS 合成即下行。"""
        try:
            self._busy = True  # 防御：直接调用 _run_turn 时也置位（barge-in 守卫依赖它）
            await self._send(protocol.response_created())
            self._buf = ""
            stream_ok = False
            stream = getattr(llm, "chat_stream", None)
            if stream is not None:
                try:
                    async for delta in stream(SYSTEM_PROMPT, user_text):
                        self._buf += delta
                        await self._drain_sentences(final=False)
                        stream_ok = True
                except Exception as e:  # 端点不支持流式/异常 → 非流式兜底
                    print(f"[llm] stream failed ({e}); fallback non-stream", file=sys.stderr)
            if not stream_ok:
                reply = await asyncio.to_thread(llm.chat, SYSTEM_PROMPT, user_text)
                self._buf = reply
            await self._drain_sentences(final=True)
            await self._send(protocol.response_done("completed"))
            print("[session] turn done", file=sys.stderr)
        except asyncio.CancelledError:
            # barge-in：在途 TTS 下行随任务取消一并停止（清队列语义）
            print("[session] turn cancelled (barge-in), pending TTS dropped", file=sys.stderr)
        finally:
            self._busy = False
            self._buf = ""

    async def _drain_sentences(self, final: bool) -> None:
        """缓冲切句：已完成的句子逐句文本+TTS 下行；barge-in（_busy=False）即停。"""
        sents = split_sentences(self._buf)
        if not sents:
            return
        pending = sents if final else sents[:-1]  # 非 final 保留末段（可能未完句）
        for s in pending:
            if not self._busy:  # 被打断 → 丢弃后续句（清 TTS 队列）
                return
            await self._send(protocol.response_text_delta(s))
            # 朗读前清洗：markdown/emoji/特殊符号不进合成器；上一行 delta 仍是原文。
            audio = await asyncio.to_thread(self.tts_fn, tts.clean_for_tts(s))
            if not self._busy:
                return
            # 记录最近朗读文本（回声文本层过滤用，滚动保留约 300 字）
            self._recent_tts_text = (self._recent_tts_text + s)[-300:]
            # 冷却截止：发送时刻 + 该段音频实际播放时长 + 残响缓冲。
            # 客户端是排队播放，若只按"发送时刻"算，队列里的音频播完时冷却已过期 → 残响触发新回合。
            dur = len(audio) / _PCM_BYTES_PER_SAMPLE / _PCM_RATE
            self._cooldown_until = max(
                self._cooldown_until, time.time() + dur + _PLAYBACK_TAIL
            )
            self._last_audio_sent_at = time.time()  # 回声冷却起点（统计用）
            await self._send(protocol.response_audio_delta(audio))
        if not final:
            self._buf = self._buf[sum(len(s) for s in pending):]
