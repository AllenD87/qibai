"""最小 MCP server：第一次启动在 tools/call 之后自杀，第二次启动正常。

专供 skill_adapter 的自愈测试：它用来证明「会话死了要丢弃重连，
而不是把同一句『stdout 关闭』复读给用户」。
"""
import json
import sys

state = sys.argv[1]
try:
    with open(state) as f:
        n = int(f.read().strip() or 0) + 1
except Exception:
    n = 1
with open(state, "w") as f:
    f.write(str(n))


def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {
            "protocolVersion": "2024-11-05",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "flaky", "version": "1"}}})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({"jsonrpc": "2.0", "id": mid, "result": {"tools": [
            {"name": "ping", "description": "", "inputSchema": {"type": "object"}}]}})
    elif method == "tools/call":
        send({"jsonrpc": "2.0", "id": mid, "result": {
            "content": [{"type": "text", "text": "pong-%d" % n}]}})
        if n == 1:
            sys.exit(0)
