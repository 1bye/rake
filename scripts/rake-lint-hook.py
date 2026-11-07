#!/usr/bin/env python3
"""Feed source-only Rake findings back through documented PostToolUse JSON."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys


def edited_files(event):
    tool = event.get("tool_name")
    arguments = event["tool_input"]
    if tool in ("Edit", "Write"):
        return [arguments["file_path"]]
    if tool == "apply_patch":
        patch = arguments["command"]
        if not isinstance(patch, str) or not patch.startswith("*** Begin Patch\n"):
            raise ValueError("unsupported apply_patch payload; run rake lint manually")
        paths = re.findall(r"^\*\*\* (?:Add File|Update File|Move to): (.+)$", patch, re.MULTILINE)
        if not paths and "*** Delete File: " not in patch:
            raise ValueError("unsupported apply_patch paths; run rake lint manually")
        return paths
    return []


def feedback(message):
    print(json.dumps({"hookSpecificOutput": {
        "hookEventName": "PostToolUse", "additionalContext": message,
    }}))


def main():
    try:
        event = json.load(sys.stdin)
        if not isinstance(event, dict):
            raise ValueError("hook input must be a JSON object")
        if event.get("hook_event_name") != "PostToolUse":
            return
        cwd = Path(event["cwd"])
        paths = sorted({(cwd / name).resolve() for name in edited_files(event)})
        messages = []
        for path in paths:
            if not path.is_file() or (path.suffix != ".rs" and path.name != "Cargo.toml"):
                continue
            result = subprocess.run(
                [os.environ.get("RAKE_LINT_BIN", "rake"), "lint", "--json", str(path)],
                cwd=cwd, text=True, capture_output=True, timeout=10, check=False,
            )
            if result.stdout:
                report = json.loads(result.stdout)
                if not isinstance(report, dict) or report.get("schema_version") != 1:
                    raise ValueError("unexpected rake lint JSON schema")
                if report.get("diagnostics"):
                    messages.append(result.stdout.strip())
                elif result.returncode:
                    messages.append(f"rake lint failed for {path}: {result.stderr}")
            else:
                messages.append(f"rake lint failed for {path}: {result.stderr}")
        if messages:
            feedback("Rake post-edit findings:\n" + "\n".join(messages))
    except (KeyError, TypeError, ValueError, OSError, subprocess.TimeoutExpired) as error:
        feedback(f"Rake lint hook could not check the edit: {error}. Run rake lint manually.")


if __name__ == "__main__":
    main()
