#!/usr/bin/env python3
import json
import sys
import time


for line in sys.stdin:
    if not line.strip():
        continue
    message = json.loads(line)
    method = message.get("method")
    sys.stderr.write(f"hanging server received {method}\n")
    sys.stderr.flush()
    time.sleep(60)
