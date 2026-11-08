#!/usr/bin/env python3
"""Exercise authentication and real browser/terminal HTTP broker exchanges."""

import concurrent.futures
import json
import os
from pathlib import Path
import selectors
import signal
import stat
import subprocess
import sys
import tempfile
import unittest
import urllib.error
import urllib.request
import urllib.parse
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "crates/rake-cli/scripts"))

from dev_bridge import BridgeError, DevBridge


class BrokerHTTP(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="rake-broker-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        (self.directory / "index.html").write_text('<html lang="en"><meta charset="utf-8"><body>test</body></html>')
        from broker_test_server import BrokerServer
        self.server = BrokerServer(self.directory)
        self.addCleanup(self.server.close)
        self.discovery = self.server.control
        self.url = self.server.url
        self.config = {"url":self.url,"token":self.server.token}
        with urllib.request.urlopen(self.url) as response:
            document = response.read().decode()
        self.assertNotIn(self.config["token"], document)
        self.assertRegex(document, r'src="/__rake/bootstrap\.js(?:\?build=\d+)?"')
        with urllib.request.urlopen(self.url + "/__rake/bootstrap.js") as response:
            self.assertNotIn(self.config["token"], response.read().decode())
        self.runtime = str(uuid.uuid4())
        self.info = {"element_inspection": "unavailable", "element_interaction": False, "protocol": 1, "runtime_id": self.runtime, "platform": "web", "commands": [], "queries": ["info"], "supported_targets": ["CurrentTab", "NewTab"], "idempotency_capacity": 1024, "completed_operations": 256, "pending_operations": 1024}

    def post(self, path, body, token=None, origin=None):
        headers = {"Content-Type": "application/json", "X-Rake-Token": self.config["token"] if token is None else token}
        if origin:
            headers["Origin"] = origin
        request = urllib.request.Request(self.url + "/__rake/" + path, json.dumps(body).encode(), headers)
        with urllib.request.urlopen(request, timeout=15) as response:
            return json.load(response)

    def test_authentication_host_origin_and_static_build_boundary(self):
        for token, origin in [("bad", None), ("é", None), (self.config["token"], "http://foreign.invalid")]:
            with self.assertRaises(urllib.error.HTTPError) as error:
                self.post("connect", {"application_id": "test", "info": self.info}, token, origin)
            self.assertEqual(error.exception.code, 403)
            error.exception.close()
        request = urllib.request.Request(self.url, headers={"Host": "foreign.invalid"})
        with self.assertRaises(urllib.error.HTTPError) as error:
            urllib.request.urlopen(request)
        self.assertEqual(error.exception.code, 403)
        error.exception.close()
        self.assertNotIn("rake-dev-bridge", (self.directory / "index.html").read_text())
        self.assertEqual(self.config["url"], self.url)

    def test_owned_protocol_roundtrip_redelivery_private_discovery_and_cli(self):
        self.post("connect", {"application_id": "test.browser", "info": self.info})
        manifest = self.discovery / f"{self.runtime}.json"
        self.assertEqual(stat.S_IMODE(self.discovery.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(manifest.stat().st_mode), 0o600)
        request = {"protocol": 1, "runtime_id": self.runtime, "key": None, "message": {"type": "query", "value": {"type": "info"}}}
        with concurrent.futures.ThreadPoolExecutor() as executor:
            future = executor.submit(self.post, "client", {"runtime_id": self.runtime, "request": request})
            delivery = self.post("poll", {"runtime_id": self.runtime})[0]
            self.assertEqual(delivery["request"], request)
            self.assertEqual(self.post("poll", {"runtime_id": self.runtime})[0], delivery)
            result = {"Ok": {"type": "info", "value": self.info}}
            self.post("reply", {"runtime_id": self.runtime, "ticket": delivery["ticket"], "result": result})
            self.assertEqual(future.result(timeout=10), result)
            # Exercise the compiled terminal HTTP adapter against the same broker.
            cli = Path(os.environ.get("CARGO_TARGET_DIR", str(Path(__file__).resolve().parents[1] / "target"))) / "debug/rake"
            self.assertTrue(cli.is_file(), "build rake-cli before running broker checks")
            process = subprocess.Popen([str(cli), "ctl", "info", "--json"], env=dict(os.environ, RAKE_CONTROL_DIR=str(self.discovery)), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            # Instance selection first validates the advertised runtime before the query.
            for _ in range(2):
                delivery = self.post("poll", {"runtime_id": self.runtime})[0]
                self.post("reply", {"runtime_id": self.runtime, "ticket": delivery["ticket"], "result": result})
            stdout, stderr = process.communicate(timeout=10)
            self.assertEqual(process.returncode, 0, stderr)
            self.assertEqual(json.loads(stdout), result["Ok"])

    def test_json_frame_bounds_shape_and_action_validation(self):
        for action, payload, content_type, status in [
            ("connect", b"{}", "text/plain", 413),
            ("connect", b"[]", "application/json", 400),
            ("connect", b"{", "application/json", 400),
            ("inspect", b"{}", "application/json", 400),
            ("inspect_unknown", b"{}", "application/json", 404),
        ]:
            request = urllib.request.Request(self.url + "/__rake/" + action, payload,
                {"Content-Type":content_type,"X-Rake-Token":self.config["token"]})
            with self.assertRaises(urllib.error.HTTPError) as error:
                urllib.request.urlopen(request, timeout=15)
            self.assertEqual(error.exception.code,status,(action,len(payload)))
            error.exception.close()
        # Reject the declared oversized frame before waiting for a body. The old
        # Python server can reset a concurrent oversized upload before its 413 arrives.
        import socket
        parsed = urllib.parse.urlsplit(self.url)
        with socket.create_connection((parsed.hostname, parsed.port), timeout=5) as connection:
            connection.sendall((f"POST /__rake/connect HTTP/1.0\r\nHost: {parsed.netloc}\r\n"
                f"Content-Type: application/json\r\nX-Rake-Token: {self.config['token']}\r\n"
                f"Content-Length: {1024 * 1024 - 1023}\r\n\r\n").encode())
            self.assertTrue(connection.recv(1024).startswith(b"HTTP/1.0 413"))
        for build in [True,-1,2**53+1,"1",None]:
            with self.assertRaises(urllib.error.HTTPError) as error:
                self.post("connect", {"application_id":"test","info":self.info,"build_id":build})
            self.assertEqual(error.exception.code,400)
            error.exception.close()


class PreviewAuthentication(unittest.TestCase):
    def test_non_ascii_preview_token_is_rejected(self):
        from types import SimpleNamespace
        from preview_socket import Socket
        handler = SimpleNamespace(headers={
            "Host": "127.0.0.1:9000", "Origin": "http://127.0.0.1:9000",
            "Sec-WebSocket-Protocol": "rake-preview-v1, rake-token.é",
        })
        with self.assertRaises(BridgeError) as error:
            Socket.upgrade(handler, "expected")
        self.assertEqual(error.exception.status, 403)


class BrokerBounds(unittest.TestCase):
    def test_capacity_and_expired_browser_discovery(self):
        with tempfile.TemporaryDirectory() as directory:
            bridge = DevBridge("http://127.0.0.1:9000", directory)
            runtime = str(uuid.uuid4())
            bridge.handle("connect", {"application_id": "test", "info": {"protocol": 1, "runtime_id": runtime, "platform": "web"}})
            bridge.tickets = {str(i): {"runtime": runtime, "created": __import__("time").monotonic(), "result": None} for i in range(32)}
            with self.assertRaises(BridgeError) as error:
                bridge.handle("client", {"runtime_id": runtime, "request": {"runtime_id": runtime, "protocol": 1}})
            self.assertEqual(error.exception.status, 429)
            bridge.instances[runtime]["seen"] -= 31
            with self.assertRaises(BridgeError):
                bridge.handle("poll", {"runtime_id": runtime})
            self.assertFalse((Path(directory) / f"{runtime}.json").exists())
            bridge.close()


if __name__ == "__main__":
    unittest.main()
