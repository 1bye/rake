"""Development-only browser broker. All mutations still execute in the application.

Private discovery files authenticate CLI callers. HTTP rejects foreign Host/Origin
values and requires a random token; queues, messages and disconnected leases expire.
"""

import json
import os
import secrets
import stat
import threading
import time
import uuid
from pathlib import Path

MAX_BYTES = 1024 * 1024


class BridgeError(Exception):
    def __init__(self, status, message):
        self.status = status
        super().__init__(message)


class DevBridge:
    def __init__(self, url, directory=None, development_state=None):
        self.url = url
        self.development_state = development_state
        self.token = secrets.token_urlsafe(32)
        self.directory = Path(directory or os.environ.get("RAKE_CONTROL_DIR", f"/tmp/rake-{os.geteuid()}"))
        self.directory.mkdir(mode=0o700, exist_ok=True)
        metadata = self.directory.lstat()
        if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077:
            raise ValueError("control directory must be an owned directory with mode 0700")
        self.condition = threading.Condition()
        self.instances = {}
        self.tickets = {}
        self.manifests = set()

    def development(self):
        if not self.development_state:
            return {"published": 0}
        try:
            return json.loads(Path(self.development_state).read_bytes()[:MAX_BYTES])
        except (OSError, ValueError):
            return {"published": 0, "phase": "starting"}

    def _expire(self):
        now = time.monotonic()
        for ticket in list(self.tickets):
            if now - self.tickets[ticket]["created"] > 60:
                del self.tickets[ticket]
        for runtime in list(self.instances):
            if now - self.instances[runtime]["seen"] > 30:
                del self.instances[runtime]
                path = self.directory / f"{runtime}.json"
                if path in self.manifests:
                    path.unlink(missing_ok=True)
                    self.manifests.remove(path)

    def close(self):
        with self.condition:
            for path in self.manifests:
                path.unlink(missing_ok=True)
            self.manifests.clear()
            self.instances.clear()
            self.condition.notify_all()

    def handle(self, action, body):
        with self.condition:
            self._expire()
            if action == "development":
                return {**self.development(), "runtimes": {key: {"build_id": value.get("build_id", 0),
                    "reload": value.get("reload"), "error": value.get("error")} for key, value in self.instances.items()}}
            if action == "connect":
                info = body.get("info", {})
                if not isinstance(info, dict):
                    raise BridgeError(400, "invalid runtime information")
                try:
                    runtime = str(uuid.UUID(info["runtime_id"]))
                except (KeyError, ValueError, TypeError, AttributeError) as error:
                    raise BridgeError(400, "invalid runtime ID") from error
                if info.get("protocol") != 1 or info.get("platform") != "web":
                    raise BridgeError(400, "unsupported browser protocol/platform")
                if runtime not in self.instances and len(self.instances) >= 32:
                    raise BridgeError(429, "browser instance capacity reached")
                application_id = body.get("application_id")
                if not isinstance(application_id, str) or not application_id or len(application_id) > 256:
                    raise BridgeError(400, "invalid application ID")
                build_id = body.get("build_id", 0)
                if not isinstance(build_id, int) or isinstance(build_id, bool) or not 0 <= build_id <= 2**53:
                    raise BridgeError(400, "invalid build ID")
                previous = self.instances.get(runtime, {})
                self.instances[runtime] = {**previous, "seen": time.monotonic(), "info": info,
                    "build_id": build_id, "reload": body.get("reload")}
                manifest = {"application_id": application_id, "pid": os.getpid(), "info": info,
                            "endpoint": {"type": "web", "url": self.url, "token": self.token}}
                path = self.directory / f"{runtime}.json"
                temporary = self.directory / f".{secrets.token_hex(12)}.tmp"
                try:
                    with open(temporary, "x", opener=lambda name, flags: os.open(name, flags, 0o600)) as file:
                        json.dump(manifest, file)
                    os.replace(temporary, path)
                finally:
                    temporary.unlink(missing_ok=True)
                self.manifests.add(path)
                return {"connected": True}

            runtime = body.get("runtime_id")
            if runtime not in self.instances:
                raise BridgeError(404, "browser instance unavailable; reconnect required")

            if action == "reload_result":
                instance = self.instances[runtime]
                instance.update(reload=body.get("reload"), error=body.get("error"))
                if body.get("error", "") and "runtime is busy" in body["error"]:
                    instance.update(attempted=0, retry_after=time.monotonic() + 1)
                return {"received": True}

            if action == "poll":
                self.instances[runtime]["seen"] = time.monotonic()
                deadline = time.monotonic() + 2
                while True:
                    deliveries = [{"ticket": ticket, "request": pending["request"]}
                                  for ticket, pending in self.tickets.items()
                                  if pending["runtime"] == runtime and pending["result"] is None]
                    if deliveries:
                        return deliveries[:1]
                    published = self.development().get("published", 0)
                    instance = self.instances[runtime]
                    if published > instance.get("build_id", 0) and published > instance.get("attempted", 0) and time.monotonic() >= instance.get("retry_after", 0):
                        instance["attempted"] = published
                        return [{"ticket": f"reload-{published}", "reload_build": published}]
                    if time.monotonic() >= deadline:
                        return []
                    self.condition.wait(min(.25, max(0, deadline - time.monotonic())))

            if action == "reply":
                pending = self.tickets.get(body.get("ticket"))
                if pending and pending["runtime"] == runtime:
                    result = body.get("result")
                    if not isinstance(result, dict) or set(result) not in ({"Ok"}, {"Err"}):
                        raise BridgeError(400, "invalid runtime reply")
                    pending["result"] = result
                    self.condition.notify_all()
                return {"received": True}

            if action == "client":
                request = body.get("request")
                if not isinstance(request, dict) or request.get("runtime_id") != runtime or request.get("protocol") != 1:
                    raise BridgeError(400, "request/runtime mismatch")
                if len(self.tickets) >= 128 or sum(pending["runtime"] == runtime for pending in self.tickets.values()) >= 32:
                    raise BridgeError(429, "development broker queue full")
                ticket = secrets.token_hex(16)
                pending = {"runtime": runtime, "request": request, "result": None, "created": time.monotonic()}
                self.tickets[ticket] = pending
                self.condition.notify_all()
                deadline = time.monotonic() + 10
                while pending["result"] is None:
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        # A transport timeout is not operation cancellation or approval.
                        raise BridgeError(504, "browser response timed out; retry with the same request key")
                    self.condition.wait(remaining)
                del self.tickets[ticket]
                return pending["result"]

            raise BridgeError(404, "unknown development bridge endpoint")
