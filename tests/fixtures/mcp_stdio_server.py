"""Disposable, credential-free MCP stdio fixture for Rust integration tests."""
import json
import os
import sys
import time

sys.stdin.reconfigure(encoding="utf-8")
sys.stdout.reconfigure(encoding="utf-8")

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
    if MODE == "hang_resource" and method == "resources/read":
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
        if MODE == "delayed_tools":
            time.sleep(0.7)
        if MODE == "bad_tool":
            response = {"tools": [{"name": ""}]}
        elif request.get("params", {}).get("cursor") == "next":
            response = {
                "tools": [
                    {
                        "name": "search_knowledge" if MODE == "knowledge" else "echo",
                        "description": "Changed echo behavior" if MODE == "changed_tool" else "Echo a string",
                        "annotations": {
                            "readOnlyHint": MODE != "changed_annotations",
                            "destructiveHint": False,
                            "openWorldHint": False,
                        },
                        "inputSchema": {
                            "type": "object",
                            "properties": {"query" if MODE == "knowledge" else "text": {"type": "integer" if MODE == "changed_schema" else "string"}},
                            "required": ["query" if MODE == "knowledge" else "text"],
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
        if MODE == "knowledge":
            response = {
                "content": [{"type": "text", "text": "synthetic excerpt"}],
                "structuredContent": {"hits": [{
                    "source_id": "fixture-source", "source_name": "Fixture notes",
                    "relative_path": "guide.md", "location": "paragraph 1",
                    "file_sha256": "a" * 64, "chunk_sha256": "b" * 64,
                    "excerpt": "synthetic excerpt"
                }]},
                "isError": False,
            }
        else:
            response = {
                "content": [
                    {"type": "text", "text": request["params"]["arguments"].get("text", "")}
                ],
                "isError": False,
            }
    elif method == "resources/read":
        response = (
            {"wrong": []}
            if MODE == "bad_resource_result"
            else {
                "contents": [
                    {
                        "uri": request["params"]["uri"],
                        "mimeType": "text/plain",
                        "text": str(os.getpid()) if MODE == "identity" else "本地测试指南",
                    }
                ]
            }
        )
    elif method == "prompts/get":
        response = (
            {"wrong": []}
            if MODE == "bad_prompt_result"
            else {
                "description": "生成本地测试摘要",
                "messages": [
                    {
                        "role": "user",
                        "content": {
                            "type": "text",
                            "text": "概括：" + request["params"]["arguments"].get("topic", ""),
                        },
                    }
                ],
            }
        )
    else:
        send({"jsonrpc": "2.0", "id": ident, "error": {"code": -32601, "message": "unknown"}})
        continue
    send({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1}})
    send({"jsonrpc": "2.0", "id": ident, "result": response})
