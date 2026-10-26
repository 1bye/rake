#!/usr/bin/env python3
"""Covers: real Fieldnotes commands, journal recovery after SIGKILL and guarded quit.
Real: native GPUI app, private local transport, files, locks and rake CLI.
Doubles: none. Requires: macOS GUI; cargo build -p rake-notes -p rake-cli.
All documents and processes belong to an isolated temporary test library.
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
    deadline = time.monotonic() + 20
    last = None
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (AssertionError, subprocess.CalledProcessError, FileNotFoundError) as error:
            last = error
        time.sleep(.05)
    raise AssertionError(f"Fieldnotes did not become ready: {last}")


def main():
    if sys.platform != "darwin":
        raise SystemExit("This real-window test requires a macOS GUI session.")
    with tempfile.TemporaryDirectory(prefix="rake-notes-", dir="/tmp") as temporary:
        directory = Path(temporary)
        control = directory / "control"
        control.mkdir(mode=0o700)
        data = directory / "data"
        env = {**os.environ, "RAKE_DEV_CONTROL": "1", "RAKE_CONTROL_DIR": str(control),
               "RAKE_NOTES_DATA_DIR": str(data), "RAKE_NOTES_AUTOSAVE_MS": "30000"}
        app = None
        with (directory / "app.log").open("w+") as log:
            def ctl(*args):
                result = subprocess.run([str(ROOT / "target/debug/rake"), "ctl", "--json", *args],
                    env=env, capture_output=True, text=True, timeout=10, check=True)
                return json.loads(result.stdout)

            def command(name, arguments):
                value = ctl("commands", "run", name, "--args", json.dumps(arguments))["reply"]["value"]
                assert value["state"] == "Committed", value
                return value["result"]

            def start():
                nonlocal app
                process = subprocess.Popen([str(ROOT / "target/debug/rake-notes")], cwd=ROOT,
                    env=env, stdout=log, stderr=log)
                app = process
                eventually(lambda: ctl("info"))
                eventually(lambda: command("notes.list", None))
                return process

            try:
                app = start()
                command("notes.save", {})
                created = command("notes.create", {"title": "Crash recovery 日本語", "body": "A durable thought 👩🏽‍💻"})
                command("notes.save", {"checkpoint_only": True})
                assert json.loads((data / "recovery.json").read_text())["library"]["selected"] == created["id"]
                assert len(json.loads((data / "library.json").read_text())["library"]["notes"]) == 3
                app.kill()
                app.wait(timeout=10)
                # A forced stop leaves discovery records; discard only this test's private records.
                for path in control.iterdir():
                    path.unlink()
                app = start()
                recovered = command("notes.list", None)
                assert any(note["id"] == created["id"] and note["title"] == created["title"] for note in recovered)
                def recovery_notice():
                    return any(node["semantic_id"] == "notes.recovery" for node in ctl("inspect", "tree")["value"]["elements"])
                eventually(recovery_notice)
                edited = command("notes.edit", {"id": created["id"], "expected_version": created["version"],
                    "body": "Last edit saved by guarded quit.\nAnother line.", "title": None})
                stale = ctl("commands", "run", "notes.edit", "--args", json.dumps({"id": created["id"],
                    "expected_version": created["version"], "body": "Must not replace", "title": None}))
                assert stale["reply"]["value"]["state"] != "Committed"
                # Force a real write failure. The dirty note must remain open after Cancel.
                recovery = data / "recovery.json"
                recovery.rename(data / "held-recovery.json")
                recovery.mkdir(mode=0o700)
                # A failed window-close save must leave both physical hosts and their data alive.
                opened = ctl("router", "open", "notebook", "--target", "new-window")["reply"]["value"]["result"]
                closing = ctl("windows", "close", f'window-{opened["window_id"]}', "--no-wait")["reply"]["value"]
                decisions = eventually(lambda: ctl("decisions", "list")["value"])
                assert decisions[0]["window_id"] == opened["window_id"]
                ctl("decisions", "respond", f'decision-{decisions[0]["id"]}', "--choice", "cancel")
                eventually(lambda: ctl("operations", "get", f'op-{closing["operation_id"]}')["value"]["state"] == {"Cancelled": "Decision"})
                assert len(ctl("windows", "list")["value"]) == 2
                assert app.poll() is None
                ctl("quit")
                decisions = eventually(lambda: ctl("decisions", "list")["value"])
                decision = decisions[0]
                assert {choice["id"] for choice in decision["choices"]} == {"save", "discard", "cancel"}
                ctl("decisions", "respond", f"decision-{decision['id']}", "--choice", "cancel")
                eventually(lambda: ctl("quit", "status")["value"]["phase"] == "cancelled")
                assert app.poll() is None
                recovery.rmdir()
                (data / "held-recovery.json").rename(recovery)
                request = ctl("quit")
                assert request["reply"]["type"] == "submitted"
                assert app.wait(timeout=20) == 0
                saved = json.loads((data / "library.json").read_text())
                note = next(note for note in saved["library"]["notes"] if note["id"] == edited["id"])
                assert note["body"] == "Last edit saved by guarded quit.\nAnother line."
                assert note["version"] == edited["version"]
                assert not list(control.iterdir()), "Graceful quit must clean up its endpoint"
                print("Fieldnotes native: typed commands, stale edits, SIGKILL recovery, recovery UI, guarded multi-window close cancellation and guarded quit passed")
            except BaseException:
                log.seek(0)
                print(log.read(), file=sys.stderr)
                raise
            finally:
                if app is not None and app.poll() is None:
                    app.kill()
                    app.wait(timeout=10)


if __name__ == "__main__":
    main()
