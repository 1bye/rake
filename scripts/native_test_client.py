"""Bounded local JSON client for this repository's native GUI test fixtures.

This is test-only IPC, not a retained broker: it reads the fixture's advertisement
and sends the exact caller envelope to its native socket.
"""
import json
from pathlib import Path
import socket


class NativeInspector:
    def __init__(self, control):
        self.control = Path(control)

    def handle(self, action, body):
        assert action == "inspect"
        request = body["request"]
        manifest = json.loads((self.control / (request["runtime_id"] + ".json")).read_bytes())
        with socket.socket(socket.AF_UNIX) as connection:
            connection.settimeout(5)
            connection.connect(manifest["endpoint"]["path"])
            connection.sendall(json.dumps(request).encode() + b"\n")
            response = bytearray()
            while b"\n" not in response:
                chunk = connection.recv(min(65536, 1024 * 1024 + 1 - len(response)))
                if not chunk:
                    raise EOFError("native fixture disconnected")
                response.extend(chunk)
                if len(response) > 1024 * 1024:
                    raise ValueError("native fixture exceeded the response bound")
            return json.loads(response.split(b"\n", 1)[0])
