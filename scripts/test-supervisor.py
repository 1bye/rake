#!/usr/bin/env python3
"""Covers build watching/failure isolation, status, cleanup and reload coordination.
Real: cargo, a compiled native fixture, process groups, filesystem and the Web broker.
Doubles: fixture application has no Rake runtime; model browser protocol for reload dispatch.
Requires: Rust/cargo/Python, Unix process groups; no display or GPU.
"""

import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
import uuid

from dev_bridge import DevBridge

spec = importlib.util.spec_from_file_location("rake_dev", Path(__file__).with_name("dev.py"))
dev = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dev)
ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "target/debug/rake"


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
                    "--target", "desktop", "--json", "--restart-without-state"], stdout=events, stderr=errors)
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
        processes = dev.Processes()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "pid"
            child = processes.start([sys.executable, "-c",
                "import subprocess,time,sys; p=subprocess.Popen([sys.executable,'-c',"
                "'import signal,time;signal.signal(signal.SIGTERM,signal.SIG_IGN);time.sleep(300)']);"
                "open(sys.argv[1],'w').write(str(p.pid));time.sleep(300)", str(path)])
            descendant = wait_for(lambda: int(path.read_text()) if path.exists() else None)
            processes.close()
            self.assertIsNotNone(child.poll())
            # A dead reparented child may briefly be a zombie; it must not be running.
            wait_for(lambda: not alive(descendant) or subprocess.run(["ps", "-o", "stat=", "-p", str(descendant)], text=True, capture_output=True).stdout.strip() in {"", "Z", "Z+"})

    def test_native_transport_retry_keeps_the_same_request_identity(self):
        import socket
        import threading
        with tempfile.TemporaryDirectory(prefix="rake-retry-", dir="/tmp") as temporary:
            path = str(Path(temporary) / "runtime.sock")
            requests = []
            with socket.socket(socket.AF_UNIX) as server:
                server.bind(path)
                server.listen()
                def serve():
                    for index in range(2):
                        connection, _ = server.accept()
                        with connection:
                            with connection.makefile("rb") as file:
                                requests.append(json.loads(file.readline()))
                            if index:
                                connection.sendall(b'{"Ok":{"type":"reload","value":{"phase":"ready"}}}\n')
                thread = threading.Thread(target=serve, daemon=True)
                thread.start()
                instance = {"info": {"runtime_id": str(uuid.uuid4())}, "endpoint": {"path": path}}
                reply = dev.exchange(instance, "command", {"type": "prepare_reload", "request": {"id": "test", "mechanism": "native_restart"}})
                thread.join(timeout=2)
                self.assertEqual(reply["phase"], "ready")
                self.assertEqual(requests[0], requests[1])
                self.assertIsNotNone(requests[0]["key"])

    def test_development_argument_errors_are_json(self):
        result = subprocess.run([str(CLI), "dev", "--unknown", "--json"], text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unrecognized", json.loads(result.stderr)["error"]["message"])

    def test_checkpoint_files_preserve_utf8_byte_bounds(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "checkpoint.json"
            value = {"draft": "é漢字" * 20000}
            dev.private_json(path, value)
            self.assertEqual(json.loads(path.read_text()), value)
            self.assertLess(path.stat().st_size, 200000)
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_build_output_is_replaced_only_after_success(self):
        import hashlib
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "src").mkdir()
            (root / "Cargo.toml").write_text('[package]\nname="build-fixture"\nversion="0.1.0"\nedition="2024"\n')
            source = root / "src/main.rs"
            source.write_text('fn main(){println!("first");}')
            subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=root, check=True, capture_output=True)
            command = [str(CLI), "build", "--manifest-path", str(root / "Cargo.toml"), "--output", str(root / "output"), "--json"]
            first = json.loads(subprocess.check_output(command, text=True, stderr=subprocess.DEVNULL))
            artifact = Path(first["artifact"])
            before = hashlib.sha256(artifact.read_bytes()).digest()
            source.write_text('fn main(){does_not_compile}')
            self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)
            self.assertEqual(hashlib.sha256(artifact.read_bytes()).digest(), before)
            source.write_text('fn main(){println!("replacement");}')
            subprocess.run(command, check=True, capture_output=True)
            self.assertNotEqual(hashlib.sha256(artifact.read_bytes()).digest(), before)
            unrelated = root / "unrelated"
            unrelated.mkdir()
            (unrelated / "keep").write_text("user data")
            refused = command.copy()
            refused[refused.index("--output") + 1] = str(unrelated)
            self.assertNotEqual(subprocess.run(refused, capture_output=True).returncode, 0)
            self.assertEqual((unrelated / "keep").read_text(), "user data")

    def test_native_backend_failure_aborts_without_stopping_the_old_host(self):
        from types import SimpleNamespace
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            backend = root / "backend"
            backend.write_text('#!/bin/sh\nexit 1\n')
            backend.chmod(0o700)
            args = SimpleNamespace(target="desktop", native_reloader=backend, json=True, restart_without_state=False)
            project = SimpleNamespace(args=args, package={"name": "test"}, root=root)
            processes = dev.Processes()
            supervisor = dev.Supervisor(project, root, processes)
            supervisor.app = processes.start([sys.executable, "-c", "import time;time.sleep(300)"])
            supervisor.instance = lambda: {"application_id": "test", "info": {"runtime_id": str(uuid.uuid4())}}
            calls = []
            original = dev.exchange
            def exchange(instance, kind, value, timeout=2, key=None):
                calls.append(value)
                if value["type"] == "reload":
                    return {"phase": "ready"}
                if value["type"] == "checkpoint":
                    return {"generation": 1}
                return {}
            dev.exchange = exchange
            try:
                with self.assertRaises(RuntimeError):
                    supervisor.publish(2, root / "replacement")
                self.assertIsNone(supervisor.app.poll())
                self.assertEqual(calls[-1]["type"], "finish_reload")
                self.assertFalse(calls[-1]["applied"])
            finally:
                dev.exchange = original
                processes.close()

    def test_immutable_web_assets_keep_the_referring_build_and_do_not_expose_traversal(self):
        import urllib.request
        import urllib.error
        import urllib.parse
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            builds = root / "builds"
            for number in (1, 2):
                output = builds / str(number)
                (output / "pkg").mkdir(parents=True)
                (output / "index.html").write_text("<!doctype html><html><head></head><script type='module' src='/loader.js'></script><script type='module'>import '/pkg/app.js';</script></html>")
                (output / "loader.js").write_text("import '/pkg/app.js';")
                (output / "pkg/app.js").write_text(f"// build {number}")
            state = root / "status.json"
            state.write_text(json.dumps({"published": 1, "directory": str(builds / "1"), "phase": "watching"}))
            env = {**os.environ, "RAKE_CONTROL_DIR": str(root / "control")}
            process = subprocess.Popen([sys.executable, str(ROOT / "scripts/serve-web.py"), "--port", "0", "--control",
                "--directory", str(builds / "1"), "--build-root", str(builds), "--development-state", str(state)],
                env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
            try:
                launch = process.stdout.readline().strip().split()[-1]
                url = urllib.parse.urlsplit(launch)
                base = f"http://{url.netloc}"
                with urllib.request.urlopen(base) as response:
                    html = response.read().decode()
                self.assertIn("/__rake/build/1/pkg/app.js", html)
                self.assertIn("/__rake/build/1/loader.js", html)
                self.assertNotIn(url.fragment.removeprefix("__rake_control="), html)
                state.write_text(json.dumps({"published": 2, "directory": str(builds / "2"), "phase": "watching"}))
                with urllib.request.urlopen(base + "/__rake/build/1/pkg/app.js") as response:
                    self.assertEqual(response.read(), b"// build 1")
                request = urllib.request.Request(base + "/pkg/app.js", headers={"Referer": base + "/__rake/build/1/loader.js"})
                with urllib.request.urlopen(request) as response:
                    self.assertEqual(response.read(), b"// build 1")
                for path in ("/__rake/build/1/%2e%2e/%2e%2e/status.json", "/__rake/build/999/pkg/app.js"):
                    try:
                        response = urllib.request.urlopen(base + path)
                    except urllib.error.HTTPError as error:
                        error.close()
                    else:
                        response.close()
                        self.fail("missing or traversal asset must not be served")
            finally:
                process.terminate()
                process.wait(timeout=5)
                process.stdout.close()

    def test_bootstrap_javascript_parses(self):
        import ast
        import shutil
        if not shutil.which("node"):
            self.skipTest("Node is optional for checking JavaScript syntax")
        module = ast.parse((ROOT / "scripts/serve-web.py").read_text())
        script = next(ast.literal_eval(item.value) for item in module.body if isinstance(item, ast.Assign)
                      and any(isinstance(target, ast.Name) and target.id == "BOOTSTRAP_JS" for target in item.targets))
        subprocess.run(["node", "--check"], input=script, check=True, capture_output=True)

    def test_web_builds_coordinate_once_per_build_and_busy_reload_does_not_starve_decisions(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            state = root / "status.json"
            state.write_text(json.dumps({"published": 1, "phase": "watching"}))
            bridge = DevBridge("http://127.0.0.1:1", root, state)
            runtime = str(uuid.uuid4())
            bridge.handle("connect", {"application_id": "test", "build_id": 1,
                "info": {"runtime_id": runtime, "protocol": 1, "platform": "web"}})
            state.write_text(json.dumps({"published": 2, "phase": "watching"}))
            delivery = bridge.handle("poll", {"runtime_id": runtime})
            self.assertEqual(delivery[0]["reload_build"], 2)
            bridge.handle("reload_result", {"runtime_id": runtime, "build_id": 2, "error": "runtime is busy"})
            bridge.tickets["decision"] = {"runtime": runtime, "created": time.monotonic(), "result": None,
                "request": {"message": {"type": "command", "value": {"type": "respond"}}}}
            self.assertIn("request", bridge.handle("poll", {"runtime_id": runtime})[0])
            bridge.tickets.clear()
            bridge.instances[runtime]["retry_after"] = 0
            self.assertEqual(bridge.handle("poll", {"runtime_id": runtime})[0]["reload_build"], 2)
            bridge.handle("reload_result", {"runtime_id": runtime, "build_id": 2, "error": None})
            self.assertEqual(bridge.instances[runtime]["attempted"], 2)
            state.write_text(json.dumps({"published": 3, "phase": "watching"}))
            self.assertEqual(bridge.handle("poll", {"runtime_id": runtime})[0]["reload_build"], 3)
            bridge.close()


if __name__ == "__main__":
    unittest.main()
