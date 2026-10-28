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

from dev_bridge import BridgeError, DevBridge


class BrokerHTTP(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="rake-broker-")
        self.directory = Path(self.temporary.name)
        (self.directory / "index.html").write_text('<html lang="en"><meta charset="utf-8"><body>test</body></html>')
        self.discovery = self.directory / "control"
        env = dict(os.environ, RAKE_CONTROL_DIR=str(self.discovery))
        self.process = subprocess.Popen([sys.executable, str(Path(__file__).with_name("serve-web.py")), "--directory", str(self.directory), "--port", "0", "--control"], env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        selector = selectors.DefaultSelector()
        selector.register(self.process.stdout, selectors.EVENT_READ)
        self.assertTrue(selector.select(10), "server failed to start")
        line = self.process.stdout.readline()
        selector.close()
        launch = urllib.parse.urlsplit(line.strip().split()[-1])
        self.url = urllib.parse.urlunsplit(launch._replace(fragment="")).rstrip("/")
        self.config = {"url": self.url, "token": launch.fragment.removeprefix("__rake_control=")}
        with urllib.request.urlopen(self.url) as response:
            document = response.read().decode()
        self.assertNotIn(self.config["token"], document)
        self.assertIn('src="/__rake/bootstrap.js"', document)
        with urllib.request.urlopen(self.url + "/__rake/bootstrap.js") as response:
            self.assertNotIn(self.config["token"], response.read().decode())
        self.runtime = str(uuid.uuid4())
        self.info = {"element_inspection": "unavailable", "element_interaction": False, "protocol": 1, "runtime_id": self.runtime, "platform": "web", "commands": [], "queries": ["info"], "supported_targets": ["CurrentTab", "NewTab"], "idempotency_capacity": 1024, "completed_operations": 256, "pending_operations": 1024}

    def tearDown(self):
        try:
            self.process.send_signal(signal.SIGINT)
            self.process.wait(timeout=10)
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait(timeout=5)
            self.process.stdout.close()
            self.temporary.cleanup()

    def post(self, path, body, token=None, origin=None):
        headers = {"Content-Type": "application/json", "X-Rake-Token": self.config["token"] if token is None else token}
        if origin:
            headers["Origin"] = origin
        request = urllib.request.Request(self.url + "/__rake/" + path, json.dumps(body).encode(), headers)
        with urllib.request.urlopen(request, timeout=15) as response:
            return json.load(response)

    def test_authentication_host_origin_and_static_build_boundary(self):
        for token, origin in [("bad", None), (self.config["token"], "http://foreign.invalid")]:
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
            cli = Path(__file__).resolve().parents[1] / "target/debug/rake"
            self.assertTrue(cli.is_file(), "build rake-cli before running broker checks")
            process = subprocess.Popen([str(cli), "ctl", "info", "--json"], env=dict(os.environ, RAKE_CONTROL_DIR=str(self.discovery)), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            # Instance selection first validates the advertised runtime before the query.
            for _ in range(2):
                delivery = self.post("poll", {"runtime_id": self.runtime})[0]
                self.post("reply", {"runtime_id": self.runtime, "ticket": delivery["ticket"], "result": result})
            stdout, stderr = process.communicate(timeout=10)
            self.assertEqual(process.returncode, 0, stderr)
            self.assertEqual(json.loads(stdout), result["Ok"])


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
