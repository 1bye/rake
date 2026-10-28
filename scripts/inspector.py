"""Authenticated inspector access to existing runtime transports, without host commands."""

import json
import os
from pathlib import Path
import socket
import stat
import urllib.error
import urllib.parse
import urllib.request
import uuid

from dev_bridge import BridgeError, MAX_BYTES


def private_read(path):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as file:
        metadata = os.fstat(file.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077:
            raise BridgeError(403, "discovery files must be owned private regular files")
        data = file.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        raise BridgeError(413, "discovery frame too large")
    return json.loads(data)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        raise BridgeError(502, "development endpoints cannot redirect")


class Inspector:
    def __init__(self, bridge, output=None, allow_native=False, cdp_url=None):
        self.bridge = bridge
        self.directory = bridge.directory
        self.output = output
        self.allow_native = allow_native
        self.cdp_url = cdp_url

    def manifest(self, runtime):
        try:
            if str(uuid.UUID(runtime)) != runtime:
                raise ValueError("noncanonical runtime ID")
            value = private_read(self.directory / f"{runtime}.json")
            if value["info"]["runtime_id"] != runtime:
                raise ValueError("discovery/runtime mismatch")
            endpoint = value["endpoint"]
            if not isinstance(value["application_id"], str) or not isinstance(value["pid"], int):
                raise ValueError("invalid application discovery")
            keys = {"local": ("path",), "web": ("url", "token")}.get(endpoint["type"])
            if keys is None or not all(isinstance(endpoint[key], str) for key in keys):
                raise ValueError("invalid transport discovery")
            return value
        except (OSError, ValueError, KeyError, TypeError, AttributeError) as error:
            raise BridgeError(404, "runtime unavailable; select a live instance") from error

    def instances(self):
        values = []
        development = self.bridge.development()
        # A bounded discovery list; no endpoint paths or session credentials cross into the page.
        for path in sorted(self.directory.glob("*.json"))[:128]:
            try:
                item = self.manifest(path.stem)
                if item["endpoint"]["type"] == "local" and not self.allow_native:
                    continue
                reload = self.bridge.instances.get(path.stem, {}).get("reload") or {}
                if item["endpoint"]["type"] == "local" and development.get("runtime_id") == path.stem:
                    reload = development.get("reload") or {}
                values.append({"application_id": item["application_id"], "pid": item["pid"],
                    "info": item["info"], "transport": item["endpoint"]["type"],
                    "restored_from": reload.get("restored_from"), "reload_phase": reload.get("phase")})
            except BridgeError:
                continue
        return values

    def handle(self, action, body):
        if action == "inspect_cdp":
            if not self.cdp_url:
                return []
            parsed = urllib.parse.urlsplit(self.cdp_url)
            if parsed.scheme != "http" or parsed.hostname != "127.0.0.1" or not parsed.port or parsed.username or parsed.password or parsed.path not in {"", "/"} or parsed.query or parsed.fragment:
                raise BridgeError(403, "CDP discovery must use IPv4 loopback HTTP")
            try:
                with urllib.request.build_opener(NoRedirect).open(self.cdp_url.rstrip("/") + "/json/list", timeout=3) as response:
                    data = response.read(MAX_BYTES + 1)
                if len(data) > MAX_BYTES:
                    raise BridgeError(413, "CDP discovery exceeds 1 MiB")
                return json.loads(data)
            except (OSError, ValueError):
                return []
        if action == "inspect_instances":
            return self.instances()
        if action == "inspect_output":
            after = body.get("after", 0)
            if not isinstance(after, int) or isinstance(after, bool) or after < 0:
                raise BridgeError(400, "invalid output cursor")
            snapshot = private_read(self.output) if self.output and self.output.is_file() else {"records": [], "dropped": 0}
            snapshot["records"] = [record for record in snapshot["records"] if record["sequence"] > after]
            return snapshot
        if action != "inspect":
            raise BridgeError(404, "unknown inspector endpoint")
        request = body.get("request")
        if not isinstance(request, dict) or request.get("protocol") != 1:
            raise BridgeError(400, "invalid runtime request")
        manifest = self.manifest(request.get("runtime_id"))
        endpoint = manifest["endpoint"]
        if endpoint["type"] == "web":
            if request["runtime_id"] in self.bridge.instances:
                return self.bridge.handle("client", {"runtime_id": request["runtime_id"], "request": request})
            parsed = urllib.parse.urlsplit(endpoint["url"])
            if parsed.scheme != "http" or parsed.hostname not in {"localhost", "127.0.0.1"} or not parsed.port or parsed.username or parsed.password or parsed.path not in {"", "/"} or parsed.query or parsed.fragment:
                raise BridgeError(403, "Web control endpoint must be loopback HTTP")
            encoded = json.dumps({"runtime_id": request["runtime_id"], "request": request}).encode()
            outgoing = urllib.request.Request(endpoint["url"].rstrip("/") + "/__rake/client", encoded,
                {"Content-Type": "application/json", "X-Rake-Token": endpoint["token"]})
            with urllib.request.build_opener(NoRedirect).open(outgoing, timeout=12) as response:
                data = response.read(MAX_BYTES + 1)
        elif endpoint["type"] == "local":
            if not self.allow_native:
                raise BridgeError(403, "application Web origins cannot access native IPC; use rake inspect")
            path = self.directory / f"{request['runtime_id']}.sock"
            if Path(endpoint["path"]) != path:
                raise BridgeError(403, "native endpoint must belong to the private control directory")
            metadata = path.lstat()
            if not stat.S_ISSOCK(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077:
                raise BridgeError(403, "native endpoint must be an owned private socket")
            encoded = json.dumps(request).encode() + b"\n"
            if len(encoded) > MAX_BYTES:
                raise BridgeError(413, "request frame too large")
            with socket.socket(socket.AF_UNIX) as connection:
                connection.settimeout(5)
                connection.connect(str(path))
                connection.sendall(encoded)
                data = bytearray()
                while b"\n" not in data:
                    chunk = connection.recv(min(65536, MAX_BYTES + 1 - len(data)))
                    if not chunk:
                        raise BridgeError(502, "runtime disconnected; delivery may have occurred")
                    data.extend(chunk)
                    if len(data) > MAX_BYTES:
                        break
        else:
            raise BridgeError(400, "unsupported runtime transport")
        if len(data) > MAX_BYTES:
            raise BridgeError(413, "runtime response too large")
        return json.loads(data)


class ProcessOutput:
    """Bounded, unassociated process text. Runtime structured diagnostics stay separate."""
    def __init__(self):
        import threading
        import time
        from collections import deque
        self.lock = threading.Lock()
        self.records = deque(maxlen=128)
        self.sequence = 0
        self.dropped = 0
        self.started = time.monotonic()

    def append(self, text, pid, stream):
        import time
        encoded = text.encode("utf-8", errors="replace")
        with self.lock:
            if len(self.records) == self.records.maxlen:
                self.dropped += 1
            self.sequence += 1
            self.records.append({"sequence": self.sequence, "elapsed_us": int((time.monotonic() - self.started) * 1000000),
                "pid": pid, "stream": stream, "text": encoded[:2048].decode("utf-8", errors="ignore"),
                "truncated": len(encoded) > 2048})

    def snapshot(self):
        with self.lock:
            return {"records": list(self.records), "dropped": self.dropped, "revision": self.sequence}
