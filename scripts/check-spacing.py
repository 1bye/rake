#!/usr/bin/env python3
"""Run the pinned Dylint library and fail on spacing warnings as well as errors."""

import argparse
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--fix", action="store_true")
args = parser.parse_args()
project = Path(__file__).resolve().parents[1]
command = ["cargo", "dylint", "--all", "--no-deps"]

if args.fix:
    command.append("--fix")

command += ["--", "--workspace", "--all-targets", "--locked", "--message-format=json"]

if args.fix:
    command += ["--allow-dirty", "--allow-staged", "--allow-no-vcs"]

result = subprocess.run(command, cwd=project, text=True, stdout=subprocess.PIPE)
failed = bool(result.returncode)
built = False
codes = {"statement_padding", "item_padding", "multiple_empty_lines", "padded_blocks"}

for line in result.stdout.splitlines():
    if not line.startswith("{"):
        continue

    event = json.loads(line)

    if event.get("reason") == "build-finished":
        built = built or event.get("success", False)

    if event.get("reason") != "compiler-message":
        continue

    message = event["message"]

    if message["level"] == "error" or (message.get("code") or {}).get("code") in codes:
        print(message.get("rendered") or message["message"])
        failed = True

if args.fix and not result.returncode and built:
    raise SystemExit(subprocess.call(["python3", __file__], cwd=project))

if not built:
    print("Dylint did not complete Cargo verification.")

raise SystemExit(int(failed or not built))
