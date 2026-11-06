#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""qibai-core 测试用 mock LLM 服务（127.0.0.1:18081）。

llm_client.rs 三个 mock_llm_chain_* 单测依赖此端点（按 model 名分支）：
- Qwen/Qwen3.6-35B-A3B-truncated  非流式 → 截断 JSON（缺右花括号，含"天气"）
- Qwen/Qwen3.6-35B-A3B-reasoning 流式 → 仅空 delta（reasoning 行为）→ content 空；
                                  非流式 → 完整 JSON 含"元气小小"
- Qwen/Qwen3.6-35B-A3B           流式 → 分片 delta 拼出含 "reply" 的 JSON

用法：python3 scripts/mock_llm_18081.py   （后台常驻，供 cargo test）
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def sse_chunk(payload: dict) -> bytes:
    return b"data: " + json.dumps(payload, ensure_ascii=False).encode("utf-8") + b"\n\n"


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        if self.path.rstrip("/") != "/v1/chat/completions":
            self.send_error(404)
            return
        length = int(self.headers.get("Content-Length", "0"))
        body = json.loads(self.rfile.read(length) or b"{}")
        model = body.get("model", "")
        stream = body.get("stream", False)

        if model.endswith("truncated"):
            # 截断 JSON：缺右花括号
            content = '{"reply":"天气很好，记得带伞'
            if stream:
                self._send_stream([{"delta": {"content": content}}])
            else:
                self._send_json({"choices": [{"message": {"role": "assistant", "content": content}}]})
        elif model.endswith("reasoning"):
            if stream:
                # reasoning 行为：只发空 delta（content 一直为空），代码据此降级非流式
                self._send_stream([{"delta": {}}, {"delta": {"reasoning_content": "思考中"}}, {"delta": {}}])
            else:
                content = '{"reply":"元气小小在呢，有什么想聊的？","emotion":"happy"}'
                self._send_json({"choices": [{"message": {"role": "assistant", "content": content}}]})
        else:
            # 默认：正常流式/非流式
            content = '{"reply":"hi from mock llm","emotion":"happy"}'
            if stream:
                pieces = ['{"rep', 'ly":"hi', ' from mock', ' llm","emo', 'tion":"happy"}']
                self._send_stream([{"delta": {"content": p}} for p in pieces])
            else:
                self._send_json({"choices": [{"message": {"role": "assistant", "content": content}}]})

    def do_GET(self):
        if self.path.rstrip("/") == "/v1/models":
            self._send_json({"data": [{"id": "mock"}]})
        else:
            self.send_error(404)

    def _send_json(self, obj: dict):
        data = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _send_stream(self, deltas):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()
        for d in deltas:
            self.wfile.write(sse_chunk({"choices": [d]}))
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()


if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18081
    print(f"[mock-llm] listening 127.0.0.1:{port}", file=sys.stderr)
    ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
