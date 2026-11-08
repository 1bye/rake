"""Real Rust dev origins for broker suites; RAKE_TEST_PYTHON_BROKER=1 selects the reference.

Cargo/wasm-bindgen/Swift are tool doubles. The listener, authentication, broker,
CLI clients and socket peers are real. Sessions and owned children are cleaned up.
"""
import json
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import sys
import time
import urllib.parse

ROOT = Path(__file__).resolve().parents[1]


class BrokerServer:
    def __init__(self, directory, native=False):
        self.session = None
        self.log = (directory / "server.log").open("w+")
        env = dict(os.environ)
        if os.environ.get("RAKE_TEST_PYTHON_BROKER") == "1":
            self.control = directory / "control"
            env["RAKE_CONTROL_DIR"] = str(self.control)
            args = [sys.executable, str(ROOT / "crates/rake-cli/scripts/serve-web.py"),
                    "--control", "--port", "0", "--directory", str(directory)]
            if native:
                args.append("--control-only")
            # Preserve the no-reverse-DNS assertion on the Python reference too.
            bootstrap = """import runpy,sys
from pathlib import Path
from unittest.mock import patch
sys.argv.pop(0)
sys.path.insert(0,str(Path(sys.argv[0]).resolve().parent))
with patch('socket.getfqdn',side_effect=AssertionError('loopback readiness must not resolve DNS')):
    runpy.run_path(sys.argv[0],run_name='__main__')
"""
            args = [sys.executable, "-c", bootstrap, *args[1:]]
        else:
            fixture = directory / "server-fixture"
            tools = fixture / "tools"
            tools.mkdir(parents=True)
            (fixture / "src").mkdir()
            (fixture / "src/main.rs").write_text("fixture")
            (fixture / "Cargo.toml").write_text("fixture")
            (fixture / "Cargo.lock").write_text("[[package]]\nname='wasm-bindgen'\nversion='0.2.100'\n")
            package = {"id": "broker-fixture", "name": "broker-fixture", "version": "1.0.0",
                "source": None, "manifest_path": str(fixture / "Cargo.toml"), "metadata": {},
                "features": {}, "dependencies": [],
                "targets": [{"name": "fixture", "kind": ["bin" if native else "cdylib"],
                             "crate_types": ["bin" if native else "cdylib"]}]}
            metadata = {"workspace_root": str(fixture), "workspace_members": [package["id"]], "packages": [package]}
            (fixture / "metadata.json").write_text(json.dumps(metadata))
            def tool(name, source):
                path = tools / name
                path.write_text("#!/bin/sh\n" + source + "\n")
                path.chmod(0o755)
            tool("cargo", 'if [ "$1" = metadata ]; then /bin/cat "$BROKER_FIXTURE/metadata.json"; else exit 7; fi')
            tool("wasm-bindgen", "printf 'wasm-bindgen 0.2.100\\n'")
            tool("xcrun", 'while [ "$1" != -o ]; do shift; done\nprintf fixture > "$2"')
            # Only the retained phase-5 preview backend needs Python.
            (tools / "python3").symlink_to(sys.executable)
            env.update(PATH=str(tools) + os.pathsep + env.get("PATH", ""), BROKER_FIXTURE=str(fixture))
            cli = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target"))) / "debug/rake"
            args = [str(cli), "dev", "--manifest-path", str(fixture / "Cargo.toml"), "--no-open", "--json", "--port", "0"]
            args += ["--preview"] if native else ["--target", "web"]
        self.process = subprocess.Popen(args, env=env, stdout=subprocess.PIPE, stderr=self.log, text=True)
        try:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                with selectors.DefaultSelector() as selector:
                    selector.register(self.process.stdout, selectors.EVENT_READ)
                    if not selector.select(max(0, deadline - time.monotonic())):
                        break
                line = self.process.stdout.readline()
                if not line:
                    break
                if line.startswith("Serving "):
                    launch = line.strip().split()[-1]
                    break
                event = json.loads(line)
                if event.get("event") in {"web_listening", "preview_listening"}:
                    launch = event["url"]
                    self.session = Path(event["session"])
                    self.control = self.session / "control"
                    break
            else:
                raise RuntimeError("broker readiness timed out")
            if "launch" not in locals():
                self.log.seek(0)
                raise RuntimeError("broker failed to start: " + self.log.read(16384))
            parsed = urllib.parse.urlsplit(launch)
            self.url = "http://" + parsed.netloc
            self.token = parsed.fragment.removeprefix("__rake_control=")
        except BaseException:
            self.close()
            raise

    def close(self):
        try:
            if self.process.poll() is None:
                self.process.terminate()
                try:
                    self.process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    self.process.kill()
                    self.process.wait(timeout=5)
        finally:
            self.process.stdout.close()
            self.log.close()
            if self.session:
                shutil.rmtree(self.session)
