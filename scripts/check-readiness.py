#!/usr/bin/env python3
"""Run bounded release checks and record exactly what was verified."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import time


ROOT = Path(__file__).resolve().parents[1]
GROUPS = {
    "portable": [
        ["cargo", "test", "-p", "rake", "--no-default-features", "--locked"],
        ["cargo", "clippy", "-p", "rake", "--no-default-features", "--all-targets", "--locked", "--", "-D", "warnings"],
    ],
    "native": [
        ["cargo", "fmt", "--all", "--check"],
        ["cargo", "clippy", "-p", "rake", "-p", "rake-foundation", "-p", "rake-desktop-example", "--all-targets", "--features", "rake/view-test-support,rake/devtools", "--locked", "--", "-D", "warnings"],
        ["cargo", "test", "-p", "rake", "--features", "view-test-support,devtools", "--lib", "--test", "view_state", "--test", "services", "--test", "platform", "--test", "web_deployment", "--locked"],
        ["cargo", "test", "-p", "rake", "--features", "experimental-view", "--doc", "--locked"],
    ],
    "web": [
        ["cargo", "clippy", "-p", "rake-foundation", "-p", "rake-routing", "-p", "rake-notes", "-p", "renamed-facade", "--lib", "--target", "wasm32-unknown-unknown", "--locked", "--", "-D", "warnings"],
        ["node", "scripts/test-notes-storage.mjs"],
        ["node", "scripts/test-platform.mjs"],
        ["node", "scripts/test-web-runtime.mjs"],
        ["node", "scripts/test-offline.mjs"],
        ["python3", "scripts/test-web-deployment.py"],
        ["cargo", "clippy", "-p", "rake-web-host-fixture", "--lib", "--target", "wasm32-unknown-unknown", "--locked", "--", "-D", "warnings"],
        ["cargo", "clippy", "-p", "rake", "--no-default-features", "--target", "wasm32-unknown-unknown", "--locked", "--", "-D", "warnings"],
    ],
    "ui": [
        ["python3", "scripts/test-app-startup.py"],
        ["python3", "scripts/test-app-layout.py"],
    ],
    "browser": [["node", "scripts/test-web-platform.mjs"]],
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("groups", choices=GROUPS, nargs="*")
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--output", type=Path, default=ROOT / "target/readiness.json")
    args = parser.parse_args()
    if args.list:
        print(json.dumps(GROUPS, indent=2))
        return 0
    if not args.groups:
        parser.error("choose at least one check group (or --list)")
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True))
    report = {"revision": revision, "dirty": dirty, "groups": args.groups, "checks": []}
    environment = os.environ.copy()
    environment.update(CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0", CARGO_INCREMENTAL="0")
    code = 0
    try:
        for group in args.groups:
            for command in GROUPS[group]:
                print(f"[{group}] {' '.join(command)}", flush=True)
                start = time.monotonic()
                try:
                    code = subprocess.run(command, cwd=ROOT, env=environment, check=False).returncode
                except OSError as error:
                    print(error, flush=True)
                    code = 127
                report["checks"].append({"group": group, "command": command, "seconds": round(time.monotonic() - start, 3), "exit_code": code})
                if code:
                    return code
        return 0
    finally:
        report["passed"] = code == 0 and len(report["checks"]) == sum(len(GROUPS[group]) for group in args.groups)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(f"Evidence: {args.output}", flush=True)


if __name__ == "__main__":
    raise SystemExit(main())
