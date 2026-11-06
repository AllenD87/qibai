# -*- coding: utf-8 -*-
"""TTS：本轮默认 NullTTS（本地合成短促正弦波 PCM），edge-tts 为可选真音色。

为什么不默认 edge-tts：沙盒/真机首次运行需要外网（Microsoft speech endpoint），
且 edge-tts 是异步 HTTP 长连接；为让「音频上行→转写→LLM→TTS 事件→下行」
冒烟零外部依赖跑通，默认走 NullTTS——它产生 16k mono s16 PCM 下行帧，
协议形态与真 TTS 完全一致，B 线 Rust 客户端无差别接收。
真机高保真链路再切 edge-tts（pip install edge-tts）。
"""
from __future__ import annotations

import math
import re
import sys
import unicodedata

import numpy as np

SAMPLE_RATE = 16000

# 句末标点（切分点）
_SENT_END = set("。！？!?；;…")
# 超长句的中间切分点（缓冲达上限后遇这些标点切）
_SOFT_END = set("，,、:： ")

# ---- 朗读前文本清洗（用户硬需求）：markdown / emoji / 特殊符号一律不读出来 ----

# 链接 [文字](url) / ![图片](url) → 只保留链接文字。
_LINK_RE = re.compile(r"!?\[([^\]]*)\]\([^)]*\)")
# 行首 markdown 标记：# 标题 / ``` ~~~ 围栏 / - * + 列表 / 1. 1、 1) 列表 / > 引用。
_LINE_MARK_RE = re.compile(r"^\s*(?:#{1,6}|```+|~~~+|[-*+]|\d+\s*[.、)]|>)\s*")
# 多空白折叠。
_WS_RE = re.compile(r"\s+")

# 允许读出来的标点（中文全角 + 英文半角常用朗读标点）。
_ALLOWED_PUNCT = set("。，！？；：、（）【】《》…—·“”‘’「」\"'.,!?;:()'-")


def clean_for_tts(text: str) -> str:
    """把 LLM 回复清洗成「可朗读」的纯文本：

    - markdown 链接 [文字](url) → 只保留「文字」；
    - 行首 # 标题 / ``` ~~~ 围栏 / - * 列表 / 1. 数字列表 / > 引用 → 去标记；
    - **加粗**、*斜体*、`行内代码`、~~~ 围栏残余 → 去符号留文字；
    - emoji / 特殊符号 → 一律剔除（只保留 L*/N* 字母数字汉字与白名单标点）；
    - 多空白折叠为单空格。

    只影响送 TTS 的文本；conversation 存储与前端气泡仍显示原文。
    """
    if not text:
        return ""
    s = text
    # 1) 链接 → 保留文字
    s = _LINK_RE.sub(r"\1", s)
    # 2) 逐行去行首 markdown 标记（围栏整行也在此被剥光，剩余 ``` 由下一步清掉）
    s = "\n".join(_LINE_MARK_RE.sub("", ln) for ln in s.splitlines())
    # 3) 去行内 markdown 残余（顺序：长符号先删）
    for tok in ("```", "~~~", "**", "`", "*"):
        s = s.replace(tok, "")
    # 4) emoji / 符号剔除：保留 Unicode 字母(L)/数字(N) + 白名单标点 + 空白
    kept = []
    for ch in s:
        if unicodedata.category(ch)[0] in ("L", "N") or ch in _ALLOWED_PUNCT or ch.isspace():
            kept.append(ch)
    s = "".join(kept)
    # 5) 多空白折叠
    return _WS_RE.sub(" ", s).strip()


def split_sentences(text: str, max_len: int = 24) -> list[str]:
    """中文回复切句：句末标点切分；缓冲超长时遇软标点强制切。

    每句独立送 TTS，首句合成即下行——砍开口延迟的关键。
    max_len=24：首句更短 → ChatTTS 首块更快出（原来 48 字首句=9s 音频要生成半天）。
    """
    text = (text or "").strip()
    if not text:
        return []
    out: list[str] = []
    buf = ""
    for ch in text:
        prev = buf[-1] if buf else ""
        buf += ch
        # 连续省略号 ……：只在第二个 … 处切一次（prev=="…" 时才切）
        if ch in _SENT_END and (ch != "…" or prev == "…"):
            out.append(buf)
            buf = ""
        elif len(buf) >= max_len and ch in _SOFT_END:
            out.append(buf)
            buf = ""
    if buf.strip():
        out.append(buf)
    return [s.strip() for s in out if s.strip()]


def null_tts_pcm(text: str, seconds: float = 0.6) -> bytes:
    """把回复文本合成一段短促正弦波（时长固定，与文字无关）。
    真 TTS 接入后：此处替换为 edge-tts 合成 → 16k s16 PCM。"""
    n = int(SAMPLE_RATE * seconds)
    t = np.arange(n) / SAMPLE_RATE
    freq = 220.0 + (hash(text) % 4) * 40.0  # 不同回复略不同频，便于日志区分
    wave = (0.3 * np.sin(2 * math.pi * freq * t) * 32767.0).astype(np.int16)
    return wave.tobytes()


def edge_tts_pcm(text: str) -> bytes:
    """edge-tts 网络流式真音色（懒导入；失败回落 null）。"""
    try:
        import asyncio as _aio
        import edge_tts  # type: ignore

        async def _synth() -> bytes:
            comm = edge_tts.Communicate(text, voice="zh-CN-XiaoxiaoNeural")
            buf = bytearray()
            async for chunk in comm.stream():
                if chunk["type"] == "audio":
                    buf.extend(chunk["data"])
            return bytes(buf)

        mp3 = _aio.run(_synth())
        # edge-tts 输出 MP3，协议要求 16k mono s16 PCM——必须解码，否则客户端当 PCM 播放=噪音。
        import subprocess
        p = subprocess.run(
            ["ffmpeg", "-hide_banner", "-loglevel", "error", "-i", "-",
             "-f", "s16le", "-ar", "16000", "-ac", "1", "-"],
            input=mp3, capture_output=True, check=True,
        )
        return p.stdout
    except Exception as e:  # 网络未通/包未装→保底
        print(f"[tts] edge-tts failed ({e}); fallback null", file=sys.stderr)
        return null_tts_pcm(text)


# ChatTTS 全局单例：模型只加载一次（2.3G 加载需 5-15s，复用后每句仅推理耗时）
_chattts: "ChatTTS.Chat | None" = None


def _load_chattts() -> "ChatTTS.Chat":
    """懒加载单例：首次调用加载模型，之后直接复用（砍掉每回合的加载延迟）。"""
    global _chattts
    if _chattts is not None:
        return _chattts
    import os
    import ChatTTS  # type: ignore

    # QIBAI_CHATTTTS_MODEL 指向本地模型目录（含 asset/ 子目录）。
    # 0.2.5 的 load(source=...) 只认 "local"/"custom"/"huggingface" 三个字面值，
    # 严禁把模型路径当 source 传：否则 download_models 三分支全不命中，
    # download_path 未初始化 → UnboundLocalError（warmup failed 的真根因）。
    # 正确姿势：source="local" + custom_path=模型目录（local 分支显式支持 custom_path），
    # 直接对 $MODEL/asset 做 sha256 校验并加载，不依赖 cwd、不依赖软链。
    model_dir = os.environ.get("QIBAI_CHATTTTS_MODEL", "")
    chat = ChatTTS.Chat()
    if model_dir:
        chat.load(source="local", custom_path=model_dir, compile=False)
    else:
        chat.load(source="local", compile=False)  # 回退 cwd/asset
    _chattts = chat
    return _chattts


def warmup_chattts() -> None:
    """预热：服务端启动即加载 ChatTTS（后台线程），首句不再吃模型加载时间。"""
    try:
        _load_chattts()
        print("[tts] chattts warmup OK（模型已加载，首句即时推理）", file=sys.stderr)
    except Exception as e:
        print(f"[tts] chattts warmup failed ({e}); 运行时将回落 edge", file=sys.stderr)


def chattts_pcm(text: str) -> bytes:
    """ChatTTS 本地 GPU 音色（复用单例；失败回落 edge→null）。"""
    try:
        import numpy as np  # noqa: F811

        chat = _load_chattts()
        wavs = chat.infer([text])
        wav = (np.asarray(wavs[0]).flatten() * 32767.0).astype(np.int16)
        return wav.tobytes()
    except Exception as e:  # 模型/包未就绪→先降级 edge（网络真音色），再 null
        print(f"[tts] chattts failed ({e}); fallback edge", file=sys.stderr)
        return edge_tts_pcm(text)


def make_tts(backend: str):
    """按开关返回 (text)->bytes 的 TTS 函数。qwen3 为配置占位（真机接入点）。

    所有后端入口统一过 [`clean_for_tts`]：markdown/emoji/特殊符号不进合成器，
    但不影响 server 下发给前端展示的 delta 原文。
    """
    if backend == "null":
        base = null_tts_pcm
    elif backend == "edge":
        base = edge_tts_pcm
    elif backend == "chattts":
        base = chattts_pcm
    elif backend == "qwen3":
        # Qwen3-TTS 真机接入点：复用 speech-to-speech 服务或 Qwen3-TTS-1.7B；
        # 沙盒未装，先走 edge 过渡。
        base = edge_tts_pcm
    else:
        base = null_tts_pcm

    def cleaned(text: str) -> bytes:
        return base(clean_for_tts(text))

    return cleaned
