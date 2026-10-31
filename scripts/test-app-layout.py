#!/usr/bin/env python3
"""Build and run the public layout smoke fixture on a desktop host."""

from __future__ import annotations

import json
import os
from pathlib import Path
import signal
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
EXAMPLE = "layout_smoke"
PASS_MARKERS = (
    "LAYOUT_SMOKE_PASS persistent-layout",
    "LAYOUT_SMOKE_PASS failed-secondary-window",
    "LAYOUT_SMOKE_PASS guarded-quit-requested",
)


def cargo_executable() -> Path:
    command = [
        "cargo",
        "build",
        "--locked",
        "-p",
        "rake",
        "--example",
        EXAMPLE,
        "--message-format=json-render-diagnostics",
    ]
    build_environment = os.environ.copy()
    build_environment.update(
        {
            "CARGO_PROFILE_DEV_DEBUG": "0",
            "CARGO_PROFILE_TEST_DEBUG": "0",
            "CARGO_INCREMENTAL": "0",
        }
    )
    process = subprocess.Popen(
        command,
        cwd=ROOT,
        env=build_environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    executable: Path | None = None
    stdout, stderr = process.communicate()
    for line in stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        target = message.get("target", {})
        if (
            message.get("reason") == "compiler-artifact"
            and target.get("name") == EXAMPLE
            and "example" in target.get("kind", [])
            and message.get("executable")
        ):
            executable = Path(message["executable"])
    if process.returncode != 0:
        sys.stderr.write(stderr)
        sys.stderr.write(stdout)
        raise SystemExit(process.returncode)
    if executable is None:
        raise RuntimeError(
            f"Cargo did not report an executable for example {EXAMPLE!r}; "
            "register the example target in crates/rake/Cargo.toml"
        )
    return executable


def run(executable: Path) -> str:
    environment = os.environ.copy()
    environment.setdefault("RUST_BACKTRACE", "1")
    process = subprocess.Popen(
        [str(executable)],
        cwd=ROOT,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    try:
        output, _ = process.communicate(timeout=30)
    except subprocess.TimeoutExpired:
        process.send_signal(signal.SIGTERM)
        try:
            output, _ = process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            output, _ = process.communicate()
        print(output, end="")
        raise RuntimeError("layout smoke application exceeded the 30-second limit")
    print(output, end="")
    if process.returncode != 0:
        raise RuntimeError(f"layout smoke application exited with {process.returncode}")
    missing = [marker for marker in PASS_MARKERS if marker not in output]
    if missing:
        raise RuntimeError(f"layout smoke output is missing PASS markers: {missing}")
    return output


def main() -> int:
    executable = cargo_executable()
    run(executable)
    print("LAYOUT_SMOKE_PASS app-layout")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
