#!/usr/bin/env python3
import json
import sys


MODES = set(sys.argv[1:])


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


for line in sys.stdin:
    if not line.strip():
        continue
    message = json.loads(line)
    method = message.get("method")
    request_id = message.get("id")

    if method == "initialize":
        if "--close" in MODES:
            break
        if "--oversized-line" in MODES:
            sys.stdout.write("x" * (1024 * 1024 + 1) + "\n")
            sys.stdout.flush()
            continue
        if "--invalid-protocol" in MODES:
            sys.stdout.write("not-json\n")
            sys.stdout.flush()
            continue
        send(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "mock", "version": "0.1.0"},
                },
            }
        )
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        if "--no-tools" in MODES:
            tools = []
        elif "--empty-tool-name" in MODES:
            tools = [
                {
                    "name": "",
                    "description": "Invalid empty tool name",
                    "inputSchema": {"type": "object"},
                }
            ]
        elif "--too-many-tools" in MODES:
            tools = [
                {
                    "name": "tool_" + str(index),
                    "description": "Bounded tool catalog fixture",
                    "inputSchema": {"type": "object"},
                }
                for index in range(129)
            ]
        else:
            tools = [
                {
                    "name": "echo_remote",
                    "description": "Echo a message through MCP",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"message": {"type": "string"}},
                        "required": ["message"],
                    },
                    "annotations": {
                        "destructiveHint": False,
                        "readOnlyHint": True,
                    },
                },
                {
                    "name": "delete_remote",
                    "description": "Destructive mock tool",
                    "inputSchema": {"type": "object"},
                    "annotations": {"destructiveHint": True},
                },
            ]
        send(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {"tools": tools},
            }
        )
    elif method == "tools/call":
        params = message.get("params", {})
        arguments = params.get("arguments", {})
        send(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": "remote: " + arguments.get("message", ""),
                        }
                    ]
                },
            }
        )
    else:
        send(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {"code": -32601, "message": "method not found"},
            }
        )
