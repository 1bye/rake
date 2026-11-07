#!/usr/bin/env python3
"""Launch the opt-in desktop native-smoke build; require a frame and clean exit.

Requires a real Windows desktop or Linux X11/Wayland display and Vulkan driver.
The timeout covers app execution, not compilation. Output is kept for CI artifacts.
"""
import argparse
import os
from pathlib import Path
import re
import subprocess

MARKER = "RAKE_DESKTOP_SMOKE_FRAME"


def smoke(command, log_path, timeout=60):
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("w", encoding="utf-8") as output:
        process = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT)
        try:
            status = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"], check=False)
            process.kill()
            process.wait()
            raise AssertionError(f"Desktop smoke exceeded {timeout}s; see {log_path}") from None
        finally:
            print(log_path.read_text(encoding="utf-8", errors="replace"), flush=True)

    log = log_path.read_text(encoding="utf-8", errors="replace")
    assert status == 0, f"Desktop exited with {status}; see {log_path}"
    assert MARKER in log.splitlines(), f"No rendered-frame marker; see {log_path}"
    assert not re.search(r"panicked at|fatal runtime error|panic in a", log), log


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    smoke([str(args.binary.resolve())], Path("target/desktop-smoke.log"))


if __name__ == "__main__":
    main()
