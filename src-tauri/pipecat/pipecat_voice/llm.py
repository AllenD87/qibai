# -*- coding: utf-8 -*-
"""LLM 客户端：OpenAI 兼容 chat completions（可配置 + 失败兜底）。

配置（环境变量）：
  QIBAI_LLM_BASE_URL  默认 http://100.92.2.152:8080/v1
  QIBAI_LLM_MODEL     默认 Qwen3.6-35B-A3B
  QIBAI_LLM_API_KEY   默认 EMPTY（内网免 key）
  QIBAI_LLM_TIMEOUT   默认 20s

任何网络/解析失败 → 返回兜底台词（不让对话链断掉），同时 stderr 记日志。
"""
from __future__ import annotations

import json
import os
import sys

import httpx

DEFAULT_BASE = os.environ.get("QIBAI_LLM_BASE_URL", "http://100.92.2.152:8080/v1")
# 模型名必须与 vLLM 注册的 served-model-name 完全一致（含 Qwen/ 前缀，否则 404）
DEFAULT_MODEL = os.environ.get("QIBAI_LLM_MODEL", "Qwen/Qwen3.6-35B-A3B")
FALLBACK_REPLY = "（LLM 暂不可达）你好，我是元气小小，我在听。"


def chat(system: str, user_text: str, timeout: float | None = None) -> str:
    url = DEFAULT_BASE.rstrip("/") + "/chat/completions"
    payload = {
        "model": DEFAULT_MODEL,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user_text},
        ],
        "temperature": 0.7,
    }
    try:
        r = httpx.post(
            url,
            json=payload,
            headers={"Authorization": f"Bearer {os.environ.get('QIBAI_LLM_API_KEY', 'EMPTY')}"},
            timeout=timeout or float(os.environ.get("QIBAI_LLM_TIMEOUT", "20")),
        )
        r.raise_for_status()
        data = r.json()
        return data["choices"][0]["message"]["content"].strip() or FALLBACK_REPLY
    except Exception as e:  # 网络/解析/超时任何失败都兜底
        print(f"[llm] fallback due to error: {e}", file=sys.stderr)
        return FALLBACK_REPLY


async def chat_stream(system: str, user_text: str, timeout: float | None = None):
    """vLLM OpenAI 兼容 SSE 流式：逐 delta yield content 增量。

    端点异常（不支持 stream / 连接失败）抛异常，由 session 回落非流式 chat()。
    """
    url = DEFAULT_BASE.rstrip("/") + "/chat/completions"
    payload = {
        "model": DEFAULT_MODEL,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user_text},
        ],
        "temperature": 0.7,
        "stream": True,
    }
    async with httpx.AsyncClient() as client:
        async with client.stream(
            "POST",
            url,
            json=payload,
            headers={"Authorization": f"Bearer {os.environ.get('QIBAI_LLM_API_KEY', 'EMPTY')}"},
            timeout=timeout or float(os.environ.get("QIBAI_LLM_TIMEOUT", "20")),
        ) as r:
            r.raise_for_status()
            async for line in r.aiter_lines():
                line = line.strip()
                if not line.startswith("data:"):
                    continue
                data = line[5:].strip()
                if data == "[DONE]":
                    break
                try:
                    obj = json.loads(data)
                    delta = obj["choices"][0].get("delta", {}).get("content")
                except Exception:  # noqa: BLE001 — SSE 心跳/坏行跳过
                    continue
                if delta:
                    yield delta
