#!/usr/bin/env python3
"""Covers: presented GPUI tree, layout/styles/source/semantics, ownership and stale IDs.
Real: native routing app, GPU/window, Unix runtime control, CLI and inspector HTTP proxy.
Doubles: none. Requires: macOS GUI session, cargo build -p rake-routing -p rake-cli.
This is an explicit display test; headless/CI suites do not substitute a fake tree.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request

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
    raise AssertionError(f"native capture did not become ready: {last}")


def main():
    if sys.platform != "darwin":
        raise SystemExit("requires a macOS GUI session; this test does not silently skip")
    with tempfile.TemporaryDirectory(prefix="rake-elements-", dir="/tmp") as temporary:
        directory = Path(temporary)
        control = directory / "control"
        control.mkdir(mode=0o700)
        env = {**os.environ, "RAKE_DEV_CONTROL": "1", "RAKE_CONTROL_DIR": str(control)}
        with (directory / "app.log").open("w+") as log:
            app = subprocess.Popen([str(ROOT / "target/debug/rake-routing")], cwd=ROOT, env=env, stdout=log, stderr=log)
            server = None
            try:
                def ctl(*arguments):
                    result = subprocess.run([str(ROOT / "target/debug/rake"), "ctl", "--json", *arguments],
                        env=env, capture_output=True, text=True, timeout=10, check=True)
                    return json.loads(result.stdout)

                def tree():
                    value = ctl("inspect", "tree")["value"]
                    return value if value["status"] == "captured" and any(node["semantic_id"] == "routing.screen" for node in value["elements"]) else None

                captured = eventually(tree)
                assert captured["omitted"] == 0
                assert len(captured["elements"]) > 15
                elements = {node["id"]: node for node in captured["elements"]}
                for node in elements.values():
                    assert node["parent"] is None or node["parent"] in elements
                    assert node["layout"]["width"] >= 0 and node["layout"]["height"] >= 0
                edit = next(node for node in elements.values() if node["semantic_id"] == "routing.edit")
                assert edit["layout"]["width"] > 0 and edit["layout"]["height"] > 0
                assert edit["styles"]["background"] is not None
                assert edit["tab_id"] is not None and edit["entry_id"] is not None
                assert elements[edit["parent"]]["semantic_id"] == "routing.screen"
                source = edit["source"]
                assert source["file"].endswith("examples/routing/src/lib.rs")
                assert '.inspect("routing.edit")' in (ROOT / source["file"]).read_text().splitlines()[source["line"] - 1]

                # The exact lookup can race an animated frame; retry from a fresh tree.
                def detail():
                    value = tree()
                    if not value:
                        return None
                    node = next(node for node in value["elements"] if node["semantic_id"] == "routing.edit")
                    value = ctl("inspect", "element", node["id"])["value"]
                    assert value == node
                    return value
                eventually(detail)
                previous = captured
                ctl("router", "open", "document", "--param", "id=B", "--target", "new-tab")
                def replaced():
                    value = tree()
                    if not value:
                        return None
                    node = next(node for node in value["elements"] if node["semantic_id"] == "routing.screen")
                    return value if node["entry_id"] != edit["entry_id"] else None
                current = eventually(replaced)
                assert all(node["entry_id"] != edit["entry_id"] for node in current["elements"])
                assert not set(elements).intersection(node["id"] for node in current["elements"])
                stale = subprocess.run([str(ROOT / "target/debug/rake"), "ctl", "--json", "inspect", "element", edit["id"]], env=env, capture_output=True, text=True, timeout=10)
                assert stale.returncode != 0 and "refresh" in stale.stderr
                # Forward the same query through the actual Elements panel's HTTP endpoint.
                server = subprocess.Popen([sys.executable, str(ROOT / "scripts/serve-web.py"), "--inspector-only", "--control", "--port", "0"], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                launch = urllib.parse.urlsplit(server.stdout.readline().strip().split()[-1])
                runtime = ctl("info")["value"]["runtime_id"]
                envelope = {"protocol": 1, "runtime_id": runtime, "key": None, "message": {"type": "query", "value": {"type": "inspect_tree"}}}
                request = urllib.request.Request(f"http://{launch.netloc}/__rake/inspect", json.dumps({"request": envelope}).encode(),
                    {"Content-Type": "application/json", "X-Rake-Token": launch.fragment.removeprefix("__rake_control=")})
                with urllib.request.urlopen(request, timeout=10) as response:
                    forwarded = json.load(response)["Ok"]
                assert forwarded["type"] == "elements" and forwarded["value"]["status"] == "captured"
                assert len(json.dumps(forwarded).encode()) < 512 * 1024
                print(f"Native Elements passed: {len(previous['elements'])} boundaries, real layout/styles/source, entry replacement, stale IDs and HTTP parity")
            except BaseException:
                log.seek(0)
                print(log.read(), file=sys.stderr)
                raise
            finally:
                for process in (server, app):
                    if process is not None:
                        process.terminate()
                        try:
                            process.wait(timeout=8)
                        except subprocess.TimeoutExpired:
                            process.kill(); process.wait(timeout=5)
                        for stream in (process.stdout, process.stderr):
                            if stream: stream.close()


if __name__ == "__main__":
    main()
