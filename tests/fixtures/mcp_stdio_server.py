"""Disposable, credential-free MCP stdio fixture for Rust integration tests."""
import json
import sys
import time

MODE = sys.argv[1] if len(sys.argv) > 1 else "normal"


def send(value):
    sys.stdout.write(json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "notifications/initialized":
        continue
    if MODE == "hang" and method == "initialize":
        time.sleep(60)
        continue
    if MODE == "oversize" and method == "initialize":
        sys.stdout.write(" " * (1024 * 1024 + 1) + "\n")
        sys.stdout.flush()
        continue
    ident = request.get("id")
    if method == "initialize":
        response = {
            "protocolVersion": "2099-01-01" if MODE == "unsupported" else "2025-06-18",
            "serverInfo": {"name": "Zi test MCP", "version": "1.0"},
            "capabilities": (
                {"resources": {}}
                if MODE == "no_tools"
                else {"tools": {}, "resources": {}, "prompts": {}}
            ),
        }
    elif method == "tools/list":
        if MODE == "bad_tool":
            response = {"tools": [{"name": ""}]}
        elif request.get("params", {}).get("cursor") == "next":
            response = {
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echo a string",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"text": {"type": "string"}},
                            "required": ["text"],
                        },
                    }
                ]
            }
        else:
            response = {
                "tools": [
                    {"name": "ping", "inputSchema": {"type": "object", "properties": {}}}
                ],
                "nextCursor": "next",
            }
    elif method == "resources/list":
        response = {"resources": [{"name": "Guide", "uri": "fixture://guide"}]}
    elif method == "prompts/list":
        response = {"prompts": [{"name": "summary"}]}
    elif method == "tools/call":
        response = {
            "content": [
                {"type": "text", "text": request["params"]["arguments"].get("text", "")}
            ],
            "isError": False,
        }
    else:
        send({"jsonrpc": "2.0", "id": ident, "error": {"code": -32601, "message": "unknown"}})
        continue
    send({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1}})
    send({"jsonrpc": "2.0", "id": ident, "result": response})
