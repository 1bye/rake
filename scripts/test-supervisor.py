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

def wait_for(predicate, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(.05)
    raise AssertionError("condition did not become true")


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


class SupervisorTests(unittest.TestCase):
    def test_web_cdp_starts_one_owned_toggle_companion_after_publication(self):
        rust_contract(self, ['web_publication_owns_server_and_single_companion_and_retains_old_build_on_failure'], 'test:project_dev')

    def test_doctor_reports_installed_targets_and_optional_node(self):
        rust_contract(self, ['doctor_matches_required_targets_and_optional_tools_without_python'], 'test:project_build')

    def test_only_dev_auto_enables_inspection_including_renamed_dependencies(self):
        rust_contract(self, ['dev_features_and_options_use_the_shared_native_supervisor'], 'bin:rake')

    def test_new_project_resolves_first_lock_but_existing_project_stays_locked(self):
        rust_contract(self, ['first_lock_resolution_and_subsequent_metadata_stay_locked'], 'test:project_build')

    def test_real_cli_watches_keeps_old_app_on_compile_failure_restarts_and_survives_crash(self):
        with tempfile.TemporaryDirectory(prefix="rake-supervisor-test-") as temporary:
            root = Path(temporary)
            (root / "src").mkdir()
            (root / "Cargo.toml").write_text('[package]\nname="supervisor-fixture"\nversion="0.1.0"\nedition="2024"\n')
            source = root / "src/main.rs"
            good = 'fn main(){println!("fixture");std::thread::sleep(std::time::Duration::from_secs(300));}'
            source.write_text(good)
            subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, check=True, capture_output=True)
            with tempfile.TemporaryFile(mode="w+") as events, tempfile.TemporaryFile(mode="w+") as errors:
                process = subprocess.Popen([str(CLI), "dev", "--manifest-path", str(root / "Cargo.toml"),
                    "--target", "desktop", "--json", "--no-open", "--restart-without-state"], stdout=events, stderr=errors)
                session = None
                try:
                    def find_session():
                        for line in os.pread(events.fileno(), 65536, 0).decode().splitlines():
                            try:
                                value = json.loads(line)
                            except ValueError:
                                continue
                            if value.get("event") == "session":
                                return Path(value["session"])
                    session = wait_for(find_session)
                    def status():
                        return json.loads((session / "status.json").read_text())
                    first = wait_for(lambda: (value if (value := status())["phase"] == "watching" else None))
                    pid = first["runtime_pid"]
                    self.assertTrue(alive(pid))
                    # Keep diagnostics outside the watched fixture; output files are otherwise inputs.
                    source.write_text('fn main(){ this_will_not_compile }')
                    failed = wait_for(lambda: (value if (value := status())["phase"] == "build_failed" else None))
                    self.assertEqual(failed["published"], first["published"])
                    self.assertTrue(alive(pid), "compiler errors must not stop the usable app")
                    source.write_text(good.replace('"fixture"', '"replacement"'))
                    second = wait_for(lambda: (value if (value := status())["phase"] == "watching" and value["published"] > first["published"] else None))
                    self.assertNotEqual(second["runtime_pid"], pid)
                    wait_for(lambda: not alive(pid))
                    os.kill(second["runtime_pid"], signal.SIGKILL)
                    wait_for(lambda: status()["phase"] == "app_exited")
                    self.assertIsNone(process.poll(), "supervisor must survive an app crash")
                    source.write_text(good.replace('"fixture"', '"after crash"'))
                    third = wait_for(lambda: (value if (value := status())["phase"] == "watching" and value["published"] > second["published"] else None))
                    stopped = subprocess.check_output([str(CLI), "dev", "status", "--session", str(session), "--json"], text=True)
                    self.assertEqual(json.loads(stopped)["runtime_pid"], third["runtime_pid"])
                finally:
                    process.send_signal(signal.SIGINT)
                    process.wait(timeout=10)
                    if session:
                        state = json.loads((session / "status.json").read_text())
                        self.assertEqual(state["phase"], "stopped")
                        self.assertFalse(alive(state["runtime_pid"]))
                        import shutil
                        shutil.rmtree(session)

    def test_owned_process_cleanup_kills_descendants_that_ignore_term(self):
        rust_contract(self, ['cleanup_kills_descendants_that_ignore_sigterm'], 'test:project_dev')

    def test_native_transport_retry_keeps_the_same_request_identity(self):
        rust_contract(self, ['native_transport_retry_keeps_the_same_request_identity'], 'bin:rake')

    def test_development_argument_errors_are_json(self):
        result = subprocess.run([str(CLI), "dev", "--unknown", "--json"], text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unrecognized", json.loads(result.stderr)["error"]["message"])

    def test_checkpoint_files_preserve_utf8_byte_bounds(self):
        rust_contract(self, ['checkpoint_files_preserve_utf8_byte_bounds'], 'bin:rake')

    def test_build_output_is_replaced_only_after_success(self):
        rust_contract(self, ['web_build_keeps_authored_index_and_rolls_back_failed_replacements'], 'test:project_build')

    def test_native_backend_failure_aborts_without_stopping_the_old_host(self):
        rust_contract(self, ['native_backend_failure_aborts_and_success_applies_without_stopping_old_host'], 'bin:rake')

    def test_immutable_web_assets_keep_the_referring_build_and_do_not_expose_traversal(self):
        rust_contract(self, ['immutable_builds_survive_publication_and_referer_pins_absolute_imports'], 'bin:rake')

    def test_bootstrap_javascript_parses(self):
        if not shutil.which("node"):
            self.skipTest("Node is optional for checking JavaScript syntax")
        subprocess.run(["node", "--check", str(ROOT / "crates/rake-cli/scripts/web-bootstrap.js")], check=True, capture_output=True)

    def test_web_builds_coordinate_once_per_build_and_busy_reload_does_not_starve_decisions(self):
        rust_contract(self, ['reload_busy_retries_and_shutdown_wakes_pending_clients'], 'bin:rake')



if __name__ == "__main__":
    unittest.main()
