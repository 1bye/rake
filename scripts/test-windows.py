#!/usr/bin/env python3
"""Covers: physical native windows, scoped CLI navigation, close and checkpointed restart.
Real: macOS GPUI hosts/views, local control transport, router/work driver and CLI.
Doubles: none. Requires: macOS GUI, cargo build -p rake-routing -p rake-cli.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def eventually(check):
    deadline = time.monotonic() + 25
    last = None
    while time.monotonic() < deadline:
        try:
            result = check()
            if result:
                return result
        except (AssertionError, subprocess.CalledProcessError) as error:
            last = error
        time.sleep(.05)
    raise AssertionError(f"native window check timed out: {last}")


def main():
    if sys.platform != "darwin":
        raise SystemExit("This real-window test requires a macOS GUI session.")
    with tempfile.TemporaryDirectory(prefix="rake-windows-", dir="/tmp") as temporary:
        directory = Path(temporary)
        control = directory / "control"
        control.mkdir(mode=0o700)
        env = {**os.environ, "RAKE_DEV_CONTROL": "1", "RAKE_CONTROL_DIR": str(control)}
        process = None
        with (directory / "application.log").open("w+") as log:
            def ctl(*args):
                result = subprocess.run([str(ROOT / "target/debug/rake"), "ctl", "--json", *args], env=env,
                    capture_output=True, text=True, timeout=15, check=True)
                return json.loads(result.stdout)

            def operation(*args):
                value = ctl(*args)["reply"]["value"]
                assert value["state"] == "Committed", value
                return value["result"]

            def snapshot():
                return ctl("snapshot")["value"]

            try:
                # Assignment is kept outside the predicate so readiness also waits for mounting.
                process = subprocess.Popen([str(ROOT / "target/debug/rake-routing")], cwd=ROOT, env=env, stdout=log, stderr=log)
                initial = eventually(lambda: (lambda state: state if state["tabs"] else None)(snapshot()))
                runtime = initial["runtime_id"]
                first = initial["windows"][0]["id"]
                second = operation("router", "open", "document", "--param", "id=A", "--target", "new-window")
                host = f'window-{second["window_id"]}'
                operation("tabs", "navigate", f'tab-{second["tab_id"]}', "document", "--param", "id=B")
                operation("router", "open", "document", "--param", "id=A", "--window", host)
                state = snapshot()
                assert len(state["windows"]) == 2, state
                assert len(ctl("windows", "get", host)["value"]["tabs"]) == 2
                assert ctl("windows", "get", f"window-{first}")["value"]["selected"] == initial["selected"]
                operation("windows", "close", f"window-{first}")
                assert process.poll() is None
                assert snapshot()["runtime_id"] == runtime
                operation("router", "open", "document", "--param", "id=C", "--target", "current-tab", "--window", host)
                third = operation("router", "open", "document", "--param", "id=D", "--target", "new-window", "--window", host)
                expected = snapshot()
                assert len(expected["windows"]) == 2
                assert all(task["origin"]["window_id"] in {item["id"] for item in expected["windows"]}
                    for task in expected["work"]["tasks"] if task["eligible"])
                ctl("reload", "prepare", "windows-restart")
                eventually(lambda: ctl("reload", "status")["value"]["phase"] == "ready")
                checkpoint = ctl("reload", "checkpoint", "com.example.rake.routing")["value"]
                assert checkpoint["version"] == 2
                assert len(checkpoint["windows"]) == 2
                ctl("reload", "abort", "windows-restart")
                ctl("quit")
                process.wait(timeout=10)
                checkpoint_path = directory / "checkpoint.json"
                checkpoint_path.write_text(json.dumps(checkpoint))
                checkpoint_path.chmod(0o600)
                env["RAKE_RELOAD_CHECKPOINT"] = str(checkpoint_path)
                process = subprocess.Popen([str(ROOT / "target/debug/rake-routing")], cwd=ROOT, env=env, stdout=log, stderr=log)
                restored = eventually(lambda: (lambda state: state if state["reload"]["phase"] == "restored" else None)(snapshot()))
                assert restored["runtime_id"] != runtime
                assert restored["tabs"] == expected["tabs"], (restored["tabs"], expected["tabs"])
                assert [(item["id"], item["selected"]) for item in restored["windows"]] == [(item["id"], item["selected"]) for item in expected["windows"]]
                assert restored["window_id"] != first
                operation("windows", "close", f'window-{third["window_id"]}')
                operation("router", "open", "document", "--param", "id=E", "--window", host)
                operation("windows", "close", host)
                assert snapshot()["windows"] == []
                assert snapshot()["tabs"] == []
                assert process.poll() is None
                reopened = operation("router", "open", "document", "--param", "id=F", "--target", "new-window")
                assert snapshot()["runtime_id"] == restored["runtime_id"]
                assert len(ctl("windows", "get", f'window-{reopened["window_id"]}')["value"]["tabs"]) == 1
                ctl("quit")
                process.wait(timeout=10)
                assert process.returncode == 0
                print("Native windows: independent hosts/histories, first/last-host close, reopening, scoped CLI and multi-window restart passed")
            except BaseException:
                log.flush()
                log.seek(0)
                print(log.read()[-12000:], file=sys.stderr)
                raise
            finally:
                if process and process.poll() is None:
                    process.terminate()
                    process.wait(timeout=10)


if __name__ == "__main__":
    main()
