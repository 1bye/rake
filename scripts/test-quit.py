#!/usr/bin/env python3
"""Covers: real native quit, response delivery, waiting/cancelling commands and cleanup.
Real: macOS GPUI window, activities, HTTP command, CLI and private Unix transport.
Doubles: a gated loopback HTTP service. Requires: macOS GUI session and
cargo build -p rake-routing -p rake-cli. Dirty guards have separate portable/CLI tests.
"""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]


def eventually(check):
    deadline = time.monotonic() + 20
    last = None
    while time.monotonic() < deadline:
        try:
            result = check()
            if result:
                return result
        except (AssertionError, subprocess.CalledProcessError) as error:
            last = error
        time.sleep(.05)
    raise AssertionError(f"native quit prerequisite did not become ready: {last}")


def scenario(policy):
    started, release = threading.Event(), threading.Event()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            started.set()
            if not release.wait(20):
                return
            try:
                self.send_response(200)
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"ok")
            except (BrokenPipeError, ConnectionResetError):
                pass  # Explicit cancellation can close the real client connection.

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="rake-quit-", dir="/tmp") as temporary:
            directory = Path(temporary)
            control = directory / "control"
            control.mkdir(mode=0o700)
            env = {**os.environ, "RAKE_DEV_CONTROL": "1", "RAKE_CONTROL_DIR": str(control)}
            with (directory / "app.log").open("w+") as log:
                app = subprocess.Popen([str(ROOT / "target/debug/rake-routing")], cwd=ROOT,
                    env=env, stdout=log, stderr=log)
                try:
                    def ctl(*arguments):
                        output = subprocess.run([str(ROOT / "target/debug/rake"), "ctl", "--json", *arguments],
                            env=env, capture_output=True, text=True, timeout=10, check=True)
                        return json.loads(output.stdout)

                    eventually(lambda: ctl("info"))

                    def quit_button():
                        tree = ctl("inspect", "tree")["value"]
                        return any(node["semantic_id"] == "rake.quit.request" and node["layout"]["width"] > 0
                            for node in tree["elements"])
                    eventually(quit_button)

                    if policy != "clean":
                        command = ctl("commands", "run", "demo.fetch", "--args",
                            json.dumps({"url": f"http://127.0.0.1:{server.server_port}/gated"}), "--no-wait")
                        operation = command["reply"]["value"]["operation_id"]
                        assert started.wait(10), "HTTP command did not start"
                    if policy == "wait":
                        quit_request = ctl("quit")
                        quit_id = quit_request["reply"]["value"]["operation_id"]
                        status = ctl("quit", "status")["value"]
                        assert status["phase"] == "waiting_for_jobs"
                        assert operation in status["pending_operations"]
                        assert app.poll() is None
                        assert ctl("operations", "cancel", f"op-{quit_id}")["value"]["accepted"]
                        eventually(lambda: ctl("quit", "status")["value"]["phase"] == "cancelled")
                        assert ctl("operations", "get", f"op-{operation}")["value"]["state"] == "Running"
                        quit_request = ctl("quit")
                        assert quit_request["reply"]["value"]["operation_id"] != quit_id
                        release.set()
                    else:
                        quit_request = ctl("quit", "--jobs", "cancel" if policy == "cancel" else "wait")
                    assert quit_request["reply"]["type"] == "submitted"
                    assert app.wait(timeout=20) == 0
                    assert not list(control.iterdir()), "native discovery/socket was not cleaned up"
                except BaseException:
                    log.seek(0)
                    print(log.read(), file=sys.stderr)
                    raise
                finally:
                    release.set()
                    if app.poll() is None:
                        app.terminate()
                        try:
                            app.wait(timeout=8)
                        except subprocess.TimeoutExpired:
                            app.kill()
                            app.wait(timeout=5)
    finally:
        release.set()
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def main():
    if sys.platform != "darwin":
        raise SystemExit("requires a macOS GUI session; this test does not silently skip")
    for policy in ("clean", "wait", "cancel"):
        scenario(policy)
    print("Native quit passed: real shell, submission delivery, job wait/cancel, cancelled quit, exit and IPC cleanup")


if __name__ == "__main__":
    main()
