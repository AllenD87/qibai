#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""栖伴 MCP 示例 server：本地备忘（memo）。

MCP（Model Context Protocol）stdio server：每行一个 JSON-RPC 2.0 消息。
实现 initialize / notifications/initialized / tools/list / tools/call，
暴露 3 个工具：memo_add / memo_list / memo_remove。

数据完全本地落盘（默认 ~/.qibai/memos.json），不上传任何内容——
符合栖伴「隐私本地优先」约束。数据文件路径可用 `--data <path>` 参数
或 QIBAI_MEMO_FILE 环境变量覆盖（测试 / 多账号隔离用）。

用法：
    python3 memo_server.py [--data <path>]
"""
import json
import os
import sys
import time
import uuid

TOOLS = [
    {
        "name": "memo_add",
        "description": "记录一条备忘（本地保存，不上传）",
        "inputSchema": {
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "备忘内容"}
            },
            "required": ["text"],
        },
    },
    {
        "name": "memo_list",
        "description": "列出全部备忘（新的在前）",
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "memo_remove",
        "description": "按 id 删除一条备忘",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "备忘 id"}
            },
            "required": ["id"],
        },
    },
]


def data_file():
    for i, arg in enumerate(sys.argv[1:]):
        if arg == "--data" and i + 1 < len(sys.argv[1:]):
            return sys.argv[1:][i + 1]
    env = os.environ.get("QIBAI_MEMO_FILE")
    if env:
        return env
    home = os.path.expanduser("~")
    return os.path.join(home, ".qibai", "memos.json")


def load_memos(path):
    try:
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
        if isinstance(data, list):
            return data
    except (OSError, ValueError):
        pass
    return []


def save_memos(path, memos):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(memos, f, ensure_ascii=False, indent=2)
    os.replace(tmp, path)


def handle_call(method, params, path):
    if method == "memo_add":
        text = (params or {}).get("text", "")
        if not isinstance(text, str) or not text.strip():
            return {"error": {"code": -32602, "message": "text 不能为空"}}
        memos = load_memos(path)
        rec = {
            "id": uuid.uuid4().hex[:12],
            "text": text.strip(),
            "created": int(time.time()),
        }
        memos.append(rec)
        save_memos(path, memos)
        return {"result": {"id": rec["id"], "count": len(memos)}}
    if method == "memo_list":
        memos = load_memos(path)
        memos = sorted(memos, key=lambda m: m.get("created", 0), reverse=True)
        return {"result": {"memos": memos, "count": len(memos)}}
    if method == "memo_remove":
        rid = (params or {}).get("id", "")
        memos = load_memos(path)
        kept = [m for m in memos if m.get("id") != rid]
        removed = len(kept) != len(memos)
        save_memos(path, kept)
        return {"result": {"removed": removed, "count": len(kept)}}
    return {"error": {"code": -32601, "message": "未知工具: %s" % method}}


def main():
    path = data_file()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        method = msg.get("method")
        rid = msg.get("id")
        # 通知（无 id）：无需响应
        if rid is None:
            continue
        if method == "initialize":
            resp = {
                "jsonrpc": "2.0",
                "id": rid,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {"listChanged": False}},
                    "serverInfo": {"name": "qibai-memo", "version": "1.0.0"},
                },
            }
        elif method == "tools/list":
            resp = {"jsonrpc": "2.0", "id": rid, "result": {"tools": TOOLS}}
        elif method == "tools/call":
            inner = handle_call(
                (msg.get("params") or {}).get("name", ""),
                (msg.get("params") or {}).get("arguments", {}),
                path,
            )
            resp = {"jsonrpc": "2.0", "id": rid}
            resp.update(inner)
        else:
            resp = {
                "jsonrpc": "2.0",
                "id": rid,
                "error": {"code": -32601, "message": "未知方法: %s" % method},
            }
        sys.stdout.write(json.dumps(resp, ensure_ascii=False) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    try:
        main()
    except BrokenPipeError:
        sys.exit(0)
