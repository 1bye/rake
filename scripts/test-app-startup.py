#!/usr/bin/env python3
"""Exercise fallible factory/root startup through the real native GPUI host.

Requires the native GUI build prerequisites and access to the desktop session.
The failure example must report its initialization error and exit unsuccessfully;
macOS NSApplication termination otherwise hides the error behind exit status zero.
"""
import json
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
env = dict(os.environ, CARGO_PROFILE_DEV_DEBUG="0", CARGO_INCREMENTAL="0")
build = subprocess.run(
    ["cargo", "build", "-p", "rake", "--example", "startup-failure", "--locked", "--message-format=json"],
    cwd=root, env=env, text=True, stdout=subprocess.PIPE, check=True,
)
artifacts = [json.loads(line) for line in build.stdout.splitlines()]
executable = next(
    artifact["executable"] for artifact in artifacts
    if artifact.get("reason") == "compiler-artifact"
    and artifact["target"]["name"] == "startup-failure"
    and artifact.get("executable")
)
result = subprocess.run([executable], cwd=root, env=env, text=True, capture_output=True, timeout=30)
assert result.returncode == 1, (result.returncode, result.stderr)
expected = "Rake startup failed: application root initialization failed:"
assert expected in result.stderr and "expected startup failure" in result.stderr, result.stderr
print("PASS: native root initialization failure reports its cause and exits with status 1")
