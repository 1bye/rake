#!/usr/bin/env python3
"""Real Text Toolkit window, captured layout, shared command and guarded host quit.

Requires macOS GUI and built rake/rake-files binaries with rake/devtools enabled.
Picker and transformation behavior have separate host/model checks; no simulated clicks.
"""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    if sys.platform != "darwin":
        raise SystemExit("requires a real macOS GUI session")
    with tempfile.TemporaryDirectory(prefix="rake-files-", dir="/tmp") as temporary:
        directory = Path(temporary)
        control = directory / "control"
        control.mkdir(mode=0o700)
        home = directory / "home"
        home.mkdir(mode=0o700)
        environment = {**os.environ, "HOME": str(home), "RAKE_DEV_CONTROL": "1", "RAKE_CONTROL_DIR": str(control)}
        with (directory / "app.log").open("w+") as log:
            app = subprocess.Popen([str(ROOT / "target/debug/rake-files")], cwd=ROOT, env=environment, stdout=log, stderr=log)
            def ctl(*arguments):
                result = subprocess.run([str(ROOT / "target/debug/rake"), "ctl", "--json", *arguments], env=environment, capture_output=True, text=True, timeout=10, check=True)
                return json.loads(result.stdout)

            try:
                deadline = time.monotonic() + 20
                captured = None
                while time.monotonic() < deadline:
                    if app.poll() is not None:
                        raise AssertionError("Text Toolkit exited before drawing")
                    try:
                        tree = ctl("inspect", "tree")["value"]
                        authored = [node for node in tree.get("elements", []) if (node.get("source") or {}).get("file", "").endswith("examples/files/src/lib.rs")]
                        if tree.get("status") == "captured" and any(node.get("layout", {}).get("width", 0) > 0 for node in authored):
                            captured = tree
                            break
                    except subprocess.CalledProcessError:
                        pass
                    time.sleep(0.05)
                assert captured is not None, "no real authored layout was captured"
                result = ctl("commands", "run", "document.statistics", "--args", "null")["reply"]["value"]
                assert result["state"] == "Committed" and result["result"] is None, result
                ctl("quit")
                assert app.wait(timeout=20) == 0
                assert not list(control.iterdir()), "native control records must be cleaned up"
                print("TEXT_TOOLKIT_PASS native drawing, shared statistics command and guarded quit")
            except BaseException:
                log.seek(0)
                print(log.read(), file=sys.stderr)
                raise
            finally:
                if app.poll() is None:
                    app.terminate()
                    try:
                        app.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        app.kill()
                        app.wait(timeout=5)


if __name__ == "__main__":
    main()
