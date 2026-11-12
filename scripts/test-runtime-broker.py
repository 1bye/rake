#!/usr/bin/env python3
"""CLI contracts after embedded Python removal; internal cases map to native Rust tests.
See crates/rake-cli/PORTING.md. HTTP/native fixtures continue running against the Rust CLI.
"""
import concurrent.futures
import io
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request
import uuid
from rust_contract_tests import rust_contract
ROOT = Path(__file__).resolve().parents[1]
CLI = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target"))) / "debug/rake"

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


class NativeInternalContracts(unittest.TestCase):
    def test_source_opening_uses_an_exact_runtime_id_and_never_accepts_paths_or_cli_options(self):
        rust_contract(self, ['source_targets_accept_only_exact_element_ids_and_captured_indices', 'source_opener_receives_exact_arguments_control_directory_and_source_root'], 'bin:rake')

    def test_cdp_discovery_rejects_non_loopback_and_defaults_to_disabled(self):
        rust_contract(self, ['loopback_endpoints_reject_credentials_paths_aliases_and_redirect_targets'], 'bin:rake')

    def test_native_successor_metadata_belongs_only_to_the_advertised_supervisor_runtime(self):
        rust_contract(self, ['native_successor_metadata_belongs_only_to_the_supervisor_runtime'], 'bin:rake')

    def test_application_web_servers_cannot_proxy_or_list_native_ipc(self):
        rust_contract(self, ['native_forwarding_keeps_the_exact_envelope_and_rejects_foreign_or_public_sockets'], 'bin:rake')

    def test_process_output_is_bounded_and_has_no_fabricated_causal_owner(self):
        rust_contract(self, ['process_output_matches_the_private_ring_and_utf8_byte_bounds', 'output_is_private_bounded_and_cursor_filtered_without_inventing_causal_ownership'], 'bin:rake')



if __name__ == "__main__":
    unittest.main()
