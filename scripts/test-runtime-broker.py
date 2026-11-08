#!/usr/bin/env python3
"""Covers: runtime broker authentication, native forwarding, discovery and process bounds.
Real: HTTP server, Unix sockets and control endpoints. Doubles: a native protocol peer.
Requires: Unix/loopback sockets; no display or browser.
"""
import concurrent.futures
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import urllib.error
import urllib.parse
import urllib.request
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "crates/rake-cli/scripts"))

from dev_bridge import BridgeError, DevBridge
from runtime_broker import RuntimeBroker, ProcessOutput


class InspectorHTTP(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="rake-inspector-", dir="/tmp")
        self.root = Path(self.temporary.name)
        from broker_test_server import BrokerServer
        self.server = BrokerServer(self.root, native=True)
        self.control = self.server.control
        self.url = self.server.url
        self.token = self.server.token

    def tearDown(self):
        self.server.close()
        self.temporary.cleanup()

    def post(self, path, body, token=None):
        request = urllib.request.Request(self.url + "/__rake/" + path, json.dumps(body).encode(),
            {"Content-Type": "application/json", "X-Rake-Token": self.token if token is None else token})
        with urllib.request.urlopen(request, timeout=10) as response:
            return json.load(response)

    def test_deprecated_frontends_are_unavailable_and_control_remains_authenticated(self):
        for filename in ["inspector", "inspector.js", "inspector-model.js", "inspector.css", "elements/", "elements/pkg/rake_inspector.js"]:
            with self.assertRaises(urllib.error.HTTPError) as error:
                urllib.request.urlopen(self.url + "/__rake/" + filename)
            self.assertEqual(error.exception.code, 410)
            error.exception.close()
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.post("inspect_instances", {}, token="wrong")
        self.assertEqual(error.exception.code, 403)
        error.exception.close()
        with self.assertRaises(urllib.error.HTTPError) as error:
            urllib.request.urlopen(self.url + "/Cargo.toml")
        self.assertEqual(error.exception.code, 404)
        error.exception.close()

    def test_cli_inspect_attaches_to_chrome_cdp_without_building(self):
        cli = Path(os.environ.get("CARGO_TARGET_DIR", str(Path(__file__).resolve().parents[1] / "target"))) / "debug/rake"
        rejected = subprocess.run([str(cli), "inspect", "--elements", "--session", str(self.root)], capture_output=True, text=True, timeout=5)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("deprecated", rejected.stderr)
        process = subprocess.Popen([str(cli), "inspect", "--session", str(self.root), "--no-open", "--json"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            event = json.loads(process.stdout.readline())
            self.assertEqual(event["event"], "cdp_listening")
            with urllib.request.urlopen(event["url"] + "/json/list") as response:
                self.assertEqual(json.load(response), [])
            self.assertFalse((self.root / "builds").exists())
        finally:
            process.terminate(); process.wait(timeout=5)
            process.stdout.close(); process.stderr.close()

    def test_native_forwarding_uses_the_exact_runtime_envelope_and_hides_credentials(self):
        runtime = str(uuid.uuid4())
        path = self.control / f"{runtime}.sock"
        with socket.socket(socket.AF_UNIX) as listener:
            listener.bind(str(path)); path.chmod(0o600); listener.listen()
            manifest = {"application_id": "test.native", "pid": os.getpid(), "info": {"runtime_id": runtime, "protocol": 1},
                "endpoint": {"type": "local", "path": str(path)}}
            advertisement = self.control / f"{runtime}.json"
            advertisement.write_text(json.dumps(manifest)); advertisement.chmod(0o600)
            discovered = self.post("inspect_instances", {})
            self.assertNotIn("endpoint", discovered[0])
            self.assertNotIn("token", json.dumps(discovered))
            request = {"protocol": 1, "runtime_id": runtime, "key": "same-retry-key", "message": {"type": "command", "value": {"type": "close", "tab": 7}}}
            observed = []
            def peer():
                connection, _ = listener.accept()
                with connection:
                    with connection.makefile("rb") as file:
                        observed.append(json.loads(file.readline()))
                    connection.sendall(b'{"Ok":{"type":"submitted","value":{"operation_id":9}}}\n')
            thread = threading.Thread(target=peer)
            thread.start()
            reply = self.post("inspect", {"request": request})
            thread.join(timeout=3)
            self.assertEqual(reply["Ok"]["value"]["operation_id"], 9)
            self.assertEqual(observed[0], request)
            # A symlink cannot redirect discovery to another private file.
            advertisement.unlink(); advertisement.symlink_to(self.root / "other")
            self.assertEqual(self.post("inspect_instances", {}), [])
            advertisement.unlink()
            manifest["endpoint"] = {}
            advertisement.write_text(json.dumps(manifest)); advertisement.chmod(0o600)
            self.assertEqual(self.post("inspect_instances", {}), [])

    def test_web_inspection_forwards_through_the_existing_browser_broker(self):
        runtime = str(uuid.uuid4())
        self.post("connect", {"application_id": "test.web", "info": {"runtime_id": runtime, "protocol": 1, "platform": "web"}, "reload": {"phase": "restored", "restored_from": "previous"}})
        self.assertEqual(self.post("inspect_instances", {})[0]["restored_from"], "previous")
        request = {"protocol": 1, "runtime_id": runtime, "key": None, "message": {"type": "query", "value": {"type": "snapshot"}}}
        with concurrent.futures.ThreadPoolExecutor() as executor:
            future = executor.submit(self.post, "inspect", {"request": request})
            delivery = self.post("poll", {"runtime_id": runtime})[0]
            self.assertEqual(delivery["request"], request)
            result = {"Ok": {"type": "data", "value": {"example": "<script>untrusted</script>"}}}
            self.post("reply", {"runtime_id": runtime, "ticket": delivery["ticket"], "result": result})
            self.assertEqual(future.result(timeout=5), result)


class ElementsHosting(unittest.TestCase):
    def test_source_opening_uses_an_exact_runtime_id_and_never_accepts_paths_or_cli_options(self):
        with tempfile.TemporaryDirectory(prefix="rake-source-host-", dir="/tmp") as temporary:
            bridge = DevBridge("http://127.0.0.1:8081", directory=Path(temporary))
            runtime = str(uuid.uuid4())
            bridge.handle("connect", {"application_id": "test.web", "info": {"runtime_id": runtime, "protocol": 1, "platform": "web"}})
            host = RuntimeBroker(bridge)
            with patch("runtime_broker.subprocess.run", return_value=subprocess.CompletedProcess([], 0, b'{"path":"/workspace/app.rs","line":3,"column":4}', b"")) as opener:
                result = host.handle("inspect_source", {"runtime_id": runtime, "id": "element-1-0", "file": "/outside.rs"})
                self.assertEqual(result["line"], 3)
                self.assertEqual(opener.call_args.args[0][1:], ["ctl", "--instance", runtime, "inspect", "source", "element-1-0", "--json"])
                self.assertEqual(opener.call_args.kwargs["env"]["RAKE_CONTROL_DIR"], temporary)
                for value in ["--session", "../app.rs", "element-1-0;touch", 1, ""]:
                    with self.assertRaises(BridgeError):
                        host.handle("inspect_source", {"runtime_id": runtime, "id": value})
                self.assertEqual(opener.call_count, 1)
                for kind in ["style", "handler"]:
                    host.handle("inspect_source", {"runtime_id": runtime, "id": "element-1-0", "target": {"kind": kind, "index": 0}})
                    self.assertEqual(opener.call_args.args[0][-3:], ["--" + kind, "0", "--json"])
                for target in [{"kind":"style","index":-1}, {"kind":"style","index":True}, {"kind":"handler","index":"--help"}, {"kind":"boundary","file":"/secret.rs"}, {"kind":"other"}]:
                    with self.assertRaises(BridgeError):
                        host.handle("inspect_source", {"runtime_id": runtime, "id": "element-1-0", "target": target})
                self.assertEqual(opener.call_count, 3)
            bridge.close()



class Bounds(unittest.TestCase):
    def test_cdp_discovery_rejects_non_loopback_and_defaults_to_disabled(self):
        with tempfile.TemporaryDirectory(prefix="rake-cdp-link-", dir="/tmp") as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            bridge = DevBridge("http://127.0.0.1:1", root)
            try:
                self.assertEqual(RuntimeBroker(bridge).handle("inspect_cdp", {}), [])
                for url in ["https://example.test", "http://127.0.0.1:9/other", "http://user@127.0.0.1:9", "http://localhost:9"]:
                    with self.assertRaises(BridgeError):
                        RuntimeBroker(bridge, cdp_url=url).handle("inspect_cdp", {})
            finally:
                bridge.close()

    def test_native_successor_metadata_belongs_only_to_the_advertised_supervisor_runtime(self):
        with tempfile.TemporaryDirectory(prefix="rake-lineage-", dir="/tmp") as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            runtime, predecessor = str(uuid.uuid4()), str(uuid.uuid4())
            status = root / "status"
            status.write_text(json.dumps({"runtime_id": runtime, "reload": {"phase": "restored", "restored_from": predecessor}}))
            for identity in [runtime, str(uuid.uuid4())]:
                path = root / f"{identity}.json"
                path.write_text(json.dumps({"application_id": "native", "pid": 1,
                    "info": {"runtime_id": identity}, "endpoint": {"type": "local", "path": str(root / f"{identity}.sock")}}))
                path.chmod(0o600)
            bridge = DevBridge("http://127.0.0.1:1", root, development_state=status)
            found = RuntimeBroker(bridge, allow_native=True).instances()
            current = next(item for item in found if item["info"]["runtime_id"] == runtime)
            other = next(item for item in found if item is not current)
            self.assertEqual(current["restored_from"], predecessor)
            self.assertIsNone(other["restored_from"])
            bridge.close()

    def test_application_web_servers_cannot_proxy_or_list_native_ipc(self):
        with tempfile.TemporaryDirectory(prefix="rake-scope-", dir="/tmp") as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            runtime = str(uuid.uuid4())
            manifest = root / f"{runtime}.json"
            manifest.write_text(json.dumps({"application_id": "native", "pid": 1,
                "info": {"runtime_id": runtime}, "endpoint": {"type": "local", "path": str(root / f"{runtime}.sock")}}))
            manifest.chmod(0o600)
            bridge = DevBridge("http://127.0.0.1:1", root)
            inspector = RuntimeBroker(bridge)
            self.assertEqual(inspector.instances(), [])
            with self.assertRaises(BridgeError) as error:
                inspector.handle("inspect", {"request": {"protocol": 1, "runtime_id": runtime, "message": {"type": "query", "value": {"type": "info"}}}})
            self.assertEqual(error.exception.status, 403)
            bridge.close()

    def test_process_output_is_bounded_and_has_no_fabricated_causal_owner(self):
        output = ProcessOutput()
        for _ in range(150):
            output.append("漢字" * 5000, 42, "stderr")
        snapshot = output.snapshot()
        self.assertEqual(len(snapshot["records"]), 128)
        self.assertEqual(snapshot["dropped"], 22)
        self.assertLess(len(json.dumps(snapshot).encode()), 1024 * 1024)
        self.assertNotIn("tab_id", snapshot["records"][0])
        self.assertTrue(snapshot["records"][0]["truncated"])
        self.assertLessEqual(len(snapshot["records"][0]["text"].encode()), 2048)


if __name__ == "__main__":
    unittest.main()
