#!/usr/bin/env python3
import json
import sys


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
        send(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "result": {
                    "tools": [
                        {
                            "name": "echo_remote",
                            "description": "Echo a message through MCP",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"message": {"type": "string"}},
                                "required": ["message"],
                            },
                        },
                        {
                            "name": "delete_remote",
                            "description": "Destructive mock tool",
                            "inputSchema": {"type": "object"},
                            "annotations": {"destructiveHint": True},
                        },
                    ]
                },
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
