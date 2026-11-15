"""A minimal client for DegenView's local MCP server: one JSON-RPC `tools/call` per request."""

import json
import os
import urllib.request

PORT = int(os.environ.get("DEGENVIEW_RECORD_PORT", "8791"))
TOKEN = os.environ["DEGENVIEW_RECORD_TOKEN"]
URL = f"http://127.0.0.1:{PORT}/mcp"
_next_id = 0


def call(name, arguments=None):
    """Call an MCP tool and return its structured result, raising on a tool or protocol error."""
    global _next_id
    _next_id += 1
    body = json.dumps({
        "jsonrpc": "2.0", "id": _next_id, "method": "tools/call",
        "params": {"name": name, "arguments": arguments or {}},
    }).encode()
    request = urllib.request.Request(URL, body, {
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream",
        "MCP-Protocol-Version": "2025-06-18",
        "Authorization": f"Bearer {TOKEN}",
    })
    with urllib.request.urlopen(request, timeout=30) as response:
        reply = json.loads(response.read())
    if "error" in reply:
        raise RuntimeError(reply["error"])
    result = reply["result"]
    if result.get("isError"):
        raise RuntimeError(result)
    if result.get("structuredContent") is not None:
        return result["structuredContent"]
    text = "".join(part.get("text", "") for part in result.get("content", []) if part.get("type") == "text")
    try:
        return json.loads(text)
    except ValueError:
        return text
