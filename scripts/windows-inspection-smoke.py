#!/usr/bin/env python3
"""Real Windows D3D11 inspection and Explorer association ownership, without installs."""
import contextlib
import ctypes
from ctypes import wintypes
import json
import os
import shutil
import signal
import socket
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
import winreg
import _winapi


def eventually(operation, timeout=30):
    deadline = time.monotonic() + timeout
    while True:
        result = operation()
        if result:
            return result
        assert time.monotonic() < deadline, "Windows inspection readiness timed out"
        time.sleep(0.05)


def windows_kernel():
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    for name, args, result in [
        ("CreateJobObjectW", [ctypes.c_void_p, wintypes.LPCWSTR], wintypes.HANDLE),
        ("AssignProcessToJobObject", [wintypes.HANDLE, wintypes.HANDLE], wintypes.BOOL),
        ("TerminateJobObject", [wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
        ("ResumeThread", [wintypes.HANDLE], wintypes.DWORD),
        ("OpenProcess", [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD], wintypes.HANDLE),
        ("IsProcessInJob", [wintypes.HANDLE, wintypes.HANDLE, ctypes.POINTER(wintypes.BOOL)], wintypes.BOOL),
        ("WaitForSingleObject", [wintypes.HANDLE, wintypes.DWORD], wintypes.DWORD),
        ("TerminateProcess", [wintypes.HANDLE, wintypes.UINT], wintypes.BOOL),
        ("CloseHandle", [wintypes.HANDLE], wintypes.BOOL),
    ]:
        function = getattr(kernel, name)
        function.argtypes, function.restype = args, result

    return kernel


@contextlib.contextmanager
def association(rake, root):
    """Launch a uniquely registered test editor from a COM helper in our own Job."""
    kernel = windows_kernel()
    protocol = "rake-editor-test-" + uuid.uuid4().hex
    key = "Software\\Classes\\" + protocol
    marker = root / "editor.pid"
    editor_code = ("import os,pathlib,sys,time; marker=pathlib.Path(sys.argv[1]); "
                   "marker.write_text(str(os.getpid())); deadline=time.monotonic()+240; "
                   "exec('while marker.exists() and time.monotonic()<deadline: time.sleep(0.1)')")
    command = subprocess.list2cmdline([sys.executable, "-c", editor_code, str(marker), "%1"])
    job = kernel.CreateJobObjectW(None, None)
    assert job, ctypes.get_last_error()
    process = thread = editor = None
    try:
        with winreg.CreateKey(winreg.HKEY_CURRENT_USER, key) as registered:
            winreg.SetValueEx(registered, "URL Protocol", 0, winreg.REG_SZ, "")
        with winreg.CreateKey(winreg.HKEY_CURRENT_USER, key + "\\shell\\open\\command") as registered:
            winreg.SetValueEx(registered, "", 0, winreg.REG_SZ, command)
        # CREATE_SUSPENDED enrolls the helper before any child code runs.
        process, thread, _, _ = _winapi.CreateProcess(
            str(rake), subprocess.list2cmdline([str(rake), "__shell-open", protocol + ":test"]),
            None, None, False, 4, None, None, subprocess.STARTUPINFO(),
        )
        assert kernel.AssignProcessToJobObject(job, process), ctypes.get_last_error()
        assert kernel.ResumeThread(thread) != 0xFFFFFFFF, ctypes.get_last_error()
        _winapi.CloseHandle(thread)
        thread = None
        assert kernel.WaitForSingleObject(process, 15000) == 0, "Explorer helper did not exit"
        assert _winapi.GetExitCodeProcess(process) == 0, "Explorer association failed"
        eventually(lambda: marker.exists())
        editor = kernel.OpenProcess(0x100401, False, int(marker.read_text()))
        assert editor, ctypes.get_last_error()
        inside = wintypes.BOOL()
        assert kernel.IsProcessInJob(editor, job, ctypes.byref(inside)), ctypes.get_last_error()
        assert not inside.value, "Explorer editor inherited the helper Job"
        assert kernel.TerminateJobObject(job, 1), ctypes.get_last_error()
        assert kernel.WaitForSingleObject(editor, 100) == 258, "Stopping the helper killed its editor"
        yield editor
    finally:
        kernel.TerminateJobObject(job, 1)
        if not editor and marker.exists():
            editor = kernel.OpenProcess(0x100401, False, int(marker.read_text()))
        if editor:
            if kernel.WaitForSingleObject(editor, 0) != 0:
                assert kernel.TerminateProcess(editor, 0), ctypes.get_last_error()
            assert kernel.WaitForSingleObject(editor, 5000) == 0, "Test editor survived cleanup"
            kernel.CloseHandle(editor)
        marker.unlink(missing_ok=True)
        if thread:
            _winapi.CloseHandle(thread)
        if process:
            _winapi.CloseHandle(process)
        kernel.CloseHandle(job)
        for suffix in ["\\shell\\open\\command", "\\shell\\open", "\\shell", ""]:
            try:
                winreg.DeleteKey(winreg.HKEY_CURRENT_USER, key + suffix)
            except FileNotFoundError:
                pass


def main():
    rake = Path(sys.argv[1]).resolve()
    kernel = windows_kernel()
    source = Path("examples/desktop/src/main.rs")
    original = source.read_bytes()
    ignored = source.parent / ".rake"
    created_ignored = not ignored.exists()
    replacement = ignored / ("inspection-" + uuid.uuid4().hex + ".rs")
    session = None
    observed = []
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    with tempfile.TemporaryDirectory(prefix="rake-inspection-") as directory:
        root = Path(directory)
        control = root / "control"
        env = dict(os.environ, RAKE_DEV_CONTROL="1", RAKE_CONTROL_DIR=str(control),
                   RAKE_DESKTOP_SMOKE_INSPECT="1", RAKE_DESKTOP_SMOKE_PREFERENCES=str(root / "preferences"),
                   RAKE_SOURCE_ROOT=str(Path.cwd()))
        application = adapter = None
        app_log, cdp_log = root / "app.log", root / "cdp.log"
        with app_log.open("w", encoding="utf-8") as app_output, cdp_log.open("w", encoding="utf-8") as cdp_output:
            try:
                application = subprocess.Popen(
                    [str(rake), "dev", "--compiler", "cargo", "--manifest-path", "examples/desktop/Cargo.toml",
                     "--package", "rake-desktop-example", "--features", "native-smoke", "--cdp", "--no-open",
                     "--port", str(port), "--json"],
                    env=env, stdout=app_output, stderr=subprocess.STDOUT,
                    creationflags=subprocess.CREATE_NEW_PROCESS_GROUP,
                )

                def events():
                    values = []
                    for line in app_log.read_text(errors="replace").splitlines():
                        try:
                            value = json.loads(line)
                            if isinstance(value, dict):
                                values.append(value)
                        except json.JSONDecodeError:
                            pass
                    return values

                session = Path(eventually(lambda: next((value["session"] for value in events()
                                                       if value.get("event") == "session"), None), 120))
                control = session / "control"

                def status():
                    result = subprocess.run([str(rake), "dev", "status", "--session", str(session), "--json"],
                                            env=env, capture_output=True, text=True, timeout=15)
                    assert result.returncode == 0, result.stderr
                    return json.loads(result.stdout)

                def published(generation):
                    state = status()
                    assert application.poll() is None, app_log.read_text(errors="replace")
                    return state if state.get("published") == generation and state.get("runtime_id") and (generation == 1 or state["runtime_id"] != runtime) else None

                state = eventually(lambda: published(1), 150)
                runtime = state["runtime_id"]
                native_cdp = json.loads((session / "cdp.json").read_text())
                with urllib.request.urlopen(native_cdp["url"] + "/json/list", timeout=10) as response:
                    assert any(target["id"] == runtime for target in json.load(response))
                manifest = next(control.glob("*.json"))
                old_pid = json.loads(manifest.read_text())["pid"]
                old_handle = kernel.OpenProcess(0x100000, False, old_pid)
                assert old_handle, ctypes.get_last_error()
                observed.append(old_handle)

                def ctl(*args):
                    result = subprocess.run([str(rake), "ctl", "--instance", runtime, *args, "--json"],
                                            env=env, capture_output=True, text=True, timeout=15)
                    assert result.returncode == 0, result.stderr
                    return json.loads(result.stdout)

                assert ctl("info")["type"] == "info"

                def capture():
                    reply = ctl("inspect", "tree")
                    assert reply["type"] == "elements", reply
                    return reply["value"] if reply["value"]["elements"] else None

                tree = eventually(capture)
                assert tree["elements"], "Real GUI did not publish inspection elements"
                detail = ctl("inspect", "element", tree["elements"][0]["id"])
                assert detail["type"] == "element", detail
                adapter = subprocess.Popen([str(rake), "inspect", "--instance", runtime, "--no-open", "--json"],
                                           env=env, stdout=cdp_output, stderr=subprocess.STDOUT)

                def ready():
                    for line in cdp_log.read_text(errors="replace").splitlines():
                        try:
                            value = json.loads(line)
                        except json.JSONDecodeError:
                            continue
                        if "url" in value and "targets" in value:
                            return value
                    return None

                server = eventually(ready)
                with urllib.request.urlopen(server["url"] + "/json/list", timeout=10) as response:
                    targets = json.load(response)
                assert targets[0]["id"] == runtime, targets
                try:
                    urllib.request.urlopen(server["url"] + "/bad-token/runtime", timeout=10)
                except urllib.error.HTTPError as error:
                    assert error.code == 403, error
                else:
                    raise AssertionError("CDP accepted an unauthenticated request")
                with association(rake, root) as editor_handle:
                    ignored.mkdir(exist_ok=True)
                    replacement.write_bytes(original + b"\n// Windows inspection reload probe.\n")
                    replacement.replace(source)
                    state = eventually(lambda: published(2), 150)
                    assert state["runtime_id"] != runtime, "Reload did not replace the native runtime"
                    assert kernel.WaitForSingleObject(old_handle, 5000) == 0, "Old GUI survived reload"
                    runtime = state["runtime_id"]
                    assert eventually(capture)["elements"]
                    cdp = json.loads((session / "cdp.json").read_text())
                    with urllib.request.urlopen(cdp["url"] + "/json/list", timeout=10) as response:
                        assert any(target["id"] == runtime for target in json.load(response))

                    listing = subprocess.run(
                        ["powershell", "-NoProfile", "-Command", "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId | ConvertTo-Json -Compress"],
                        capture_output=True, text=True, timeout=20, check=True,
                    )
                    rows = json.loads(listing.stdout)
                    owned = {application.pid}
                    for _ in rows:
                        children = {row["ProcessId"] for row in rows if row["ParentProcessId"] in owned}
                        if children <= owned:
                            break
                        owned |= children
                    assert len(owned) >= 3, "Expected supervisor, GUI and CDP adapter"
                    for pid in owned:
                        handle = kernel.OpenProcess(0x100000, False, pid)
                        if handle:
                            observed.append(handle)
                    application.send_signal(signal.CTRL_BREAK_EVENT)
                    assert application.wait(timeout=30) == 0
                    assert status()["phase"] == "stopped"
                    assert not list(control.glob("*.json")), "GUI discovery survived shutdown"
                    for handle in observed:
                        assert kernel.WaitForSingleObject(handle, 5000) == 0, "A supervised Windows process survived shutdown"
                    assert "RAKE_DESKTOP_SMOKE_FRAME" in app_log.read_text(errors="replace")
                    assert kernel.WaitForSingleObject(editor_handle, 100) == 258, "Stopping dev killed the source editor"
                    print("Windows desktop dev/CDP, real GUI inspect, reload, shutdown and editor Job survival passed")
            finally:
                for process in [adapter, application]:
                    if process is not None and process.poll() is None:
                        process.kill()
                        process.wait(timeout=10)
                source.write_bytes(original)
                replacement.unlink(missing_ok=True)
                if created_ignored and ignored.exists():
                    ignored.rmdir()
                for handle in observed:
                    kernel.CloseHandle(handle)
                for log in [app_log, cdp_log]:
                    print(log.read_text(errors="replace"), flush=True)
                if session is not None:
                    shutil.rmtree(session)


if __name__ == "__main__":
    main()
