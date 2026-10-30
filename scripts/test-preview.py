#!/usr/bin/env python3
"""Covers: preview routing, stale references, bounded input and helper lifecycle.
Real: Python preview bridge and subprocess ownership. Doubles: inspector replies; no GPU.
Requires: Python 3.11+. Native capture/browser verification is documented separately.
"""
import io
from collections import deque
import threading
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request

from dev_bridge import BridgeError
from preview import Preview, Capture
from preview_socket import Socket


class Inspector:
    def __init__(self):
        self.requests = []

    def handle(self, action, body):
        self.requests.append(body["request"])
        return {"Ok": {"type": "element", "value": {"source": {"file": "src/main.rs", "line": 12}}}}


class PreviewTests(unittest.TestCase):
    def setUp(self):
        self.inspector = Inspector()
        self.preview = Preview(self.inspector, "/unused")
        self.window = {"window": "window-1", "native_id": 42, "generation": "current",
                       "frame": {"width": 600, "height": 428}, "content": {"x": 0, "y": 28, "width": 600, "height": 400}}
        self.preview.references.append(("frame", "runtime", self.window, time.monotonic()))

    def tearDown(self):
        self.preview.close()

    def test_selection_keeps_source_and_exact_window_without_a_mutation(self):
        result = self.preview.handle("preview_select", {"reference": "frame", "x": 100.4, "y": 80})
        self.assertEqual(result["source"]["file"], "src/main.rs")
        request = self.inspector.requests[0]
        self.assertEqual(request["message"]["type"], "query")
        self.assertEqual(request["message"]["value"]["point"], {"window": "window-1", "native_id": 42, "generation": "current", "x": 100, "y": 80})

    def test_click_is_submitted_once_without_a_retry_key(self):
        self.preview.handle("preview_input", {"reference": "frame", "x": 12, "y": 13, "input": {"type": "click"}})
        self.assertEqual(len(self.inspector.requests), 1)
        request = self.inspector.requests[0]
        self.assertIsNone(request["key"])
        self.assertEqual(request["message"]["type"], "command")
        self.assertEqual(request["message"]["value"]["input"], {"type": "click"})

    def test_invalid_expired_or_foreign_positions_never_reach_runtime(self):
        for change in ({"reference": "other"}, {"x": float("nan")}, {"x": True}, {"y": -1}, {"x": 600}):
            with self.subTest(change=change), self.assertRaises(BridgeError):
                self.preview.handle("preview_select", {"reference": "frame", "x": 20, "y": 30, **change})
        self.preview.references[0] = ("frame", "runtime", self.window, time.monotonic() - 6)
        with self.assertRaises(BridgeError):
            self.preview.handle("preview_select", {"reference": "frame", "x": 20, "y": 30})
        self.assertEqual(self.inspector.requests, [])

    def test_scroll_is_bounded_and_unsupported_input_is_rejected(self):
        for input in ({"type": "keyboard"}, {"type": "scroll", "dx": 0, "dy": 2001}, {"type": "scroll", "dx": 0, "dy": float("inf")}):
            with self.subTest(input=input), self.assertRaises(BridgeError):
                self.preview.handle("preview_input", {"reference": "frame", "x": 20, "y": 30, "input": input})
        self.assertEqual(self.inspector.requests, [])

    def test_closing_preview_terminates_only_its_helper_and_drops_references(self):
        child = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(60)"])
        class Child:
            def close(self):
                child.terminate()
                child.wait(timeout=2)
        self.preview.captures['h264'] = Child()
        self.preview.close()
        self.assertIsNotNone(child.poll())
        self.assertEqual(len(self.preview.references), 0)

    def test_truncated_and_oversized_capture_frames_fail_explicitly(self):
        with self.assertRaises(EOFError):
            Capture.read(io.BytesIO(b"x"), 2)

    def test_fractional_scroll_uses_viewport_identity_and_preserves_phase(self):
        self.preview.handle('preview_input', {'reference':'frame', 'x':20, 'y':30,
            'input':{'type':'wheel','dx':1,'dy':-31,'phase':'moved'}})
        value = self.inspector.requests[-1]['message']['value']
        self.assertEqual(value['point']['geometry'], [38400,27392,0,1792,38400,25600])
        self.assertEqual(value['input']['dy'], -31)
        for bad in ({'dy':128001}, {'phase':'unknown'}, {'dy':1.5}):
            with self.assertRaises(BridgeError):
                self.preview.handle('preview_input', {'reference':'frame','x':20,'y':30,
                    'input':{'type':'wheel','dx':0,'dy':1,'phase':'started',**bad}})

    def test_video_join_starts_at_keyframe_and_releases_viewer_on_disconnect(self):
        capture = Capture.__new__(Capture)
        capture.condition = threading.Condition()
        capture.sequence, capture.viewers = 0, 0
        capture.closed, capture.error = False, None
        capture.packets = deque()
        capture.config = b'config'
        def keyframe():
            # A viewer must never receive the delta preceding its first IDR.
            for n, kind, data in [(1, {}, b'old-delta'), (2, {'key':True}, b'key'), (3, {}, b'delta')]:
                capture.packets.append((n, time.monotonic(), {'kind':'frame', **kind}, data))
            capture.sequence = 3
        capture.keyframe = keyframe
        writes = []
        class Writer:
            def write(self, data):
                writes.append(data)
                if len(writes) == 3:
                    raise BrokenPipeError()
        class Handler:
            wfile = Writer()
        with self.assertRaises(BrokenPipeError):
            capture.stream(Handler())
        self.assertEqual(writes, [b'config', b'key', b'delta'])
        self.assertEqual(capture.viewers, 0)

    def test_disconnect_during_video_headers_releases_encoder(self):
        class Encoder:
            condition = threading.Condition()
            viewers, error, closed = 0, None, False
            def close(self): self.closed = True
        class Connection:
            def setsockopt(self, *args): pass
            def settimeout(self, *args): pass
        class Handler:
            connection = Connection()
            def send_response(self, status): raise BrokenPipeError()
        encoder = Encoder()
        self.preview.captures['h264'] = encoder
        self.preview.state = lambda: {'window': self.window}
        self.preview.video(Handler(), {'codec':'h264'})
        self.assertEqual(encoder.viewers, 0)
        self.assertTrue(encoder.closed)
        self.assertEqual(self.preview.captures, {})

    def test_websocket_reads_masked_text_and_rejects_fragments_and_large_frames(self):
        class Handler:
            wfile = io.BytesIO()
        handler = Handler()
        payload = b'{"id":1,"action":"preview_state"}'
        mask = bytes([1,2,3,4])
        frame = bytes([0x81,0x80 | len(payload)]) + mask + bytes(v ^ mask[i%4] for i,v in enumerate(payload))
        handler.rfile = io.BytesIO(frame)
        self.assertEqual(Socket(handler).read()['id'], 1)
        for invalid in (b'\x01\x80', b'\x81\x00', b'\x81\xfe' + struct.pack('>H', 16385)):
            handler.rfile = io.BytesIO(invalid)
            with self.assertRaises(ValueError):
                Socket(handler).read()

    def test_preview_flag_rejects_web_and_non_dev_commands(self):
        for arguments in (["dev", "--target", "web"], ["build"]):
            result = subprocess.run([sys.executable, str(Path(__file__).with_name("dev.py")), *arguments, "--preview"], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("--preview requires", result.stderr)

    def test_preview_http_requires_token_and_same_origin(self):
        with tempfile.TemporaryDirectory(prefix="rake-preview-http-", dir="/tmp") as temporary:
            root = Path(temporary)
            state = root / "state.json"
            state.write_text('{}')
            process = subprocess.Popen([sys.executable, str(Path(__file__).with_name("serve-web.py")),
                "--control", "--inspector-only", "--port", "0", "--development-state", str(state),
                "--build-root", str(root), "--preview-helper", "/unused"],
                env={**os.environ, "RAKE_CONTROL_DIR": str(root / "control")}, stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, text=True)
            try:
                launch = urllib.parse.urlsplit(process.stdout.readline().split()[-1])
                url = f"http://{launch.netloc}"
                token = launch.fragment.removeprefix("__rake_control=")
                for credential, origin in (("wrong", url), (token, "https://foreign.invalid")):
                    request = urllib.request.Request(url + "/__rake/preview_state", b'{}',
                        {"Content-Type": "application/json", "X-Rake-Token": credential, "Origin": origin})
                    with self.assertRaises(urllib.error.HTTPError) as error:
                        urllib.request.urlopen(request, timeout=3)
                    self.assertEqual(error.exception.code, 403)
                    error.exception.close()
                with self.assertRaises(urllib.error.HTTPError) as error:
                    urllib.request.urlopen(url + '/__rake/preview_control', timeout=3)
                self.assertEqual(error.exception.code, 403)
                error.exception.close()
                request = urllib.request.Request(url + "/__rake/preview_state", b'{}',
                    {"Content-Type": "application/json", "X-Rake-Token": token, "Origin": url})
                with urllib.request.urlopen(request, timeout=3) as response:
                    self.assertIn("Waiting for the native", json.load(response)["error"])
                with urllib.request.urlopen(url + "/__rake/preview", timeout=3) as response:
                    self.assertNotIn(token, response.read().decode())
                    self.assertIn("frame-ancestors 'none'", response.headers["Content-Security-Policy"])
            finally:
                process.terminate()
                process.wait(timeout=5)
                process.stdout.close()
                process.stderr.close()


if __name__ == "__main__":
    unittest.main()
