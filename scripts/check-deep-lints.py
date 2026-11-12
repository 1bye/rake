#!/usr/bin/env python3
"""Require a clean deep report for the workspace and every top-level example."""
import json
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
failed = False
for path in [root, *sorted(manifest.parent for manifest in (root / "examples").glob("*/Cargo.toml"))]:
    result = subprocess.run(
        ["cargo", "run", "-p", "rake-app-cli", "--locked", "--", "lint",
         str(path), "--deep", "--json"],
        cwd=root, text=True, capture_output=True, check=False,
    )
    if result.returncode:
        print(result.stderr or result.stdout, file=sys.stderr)
        failed = True
        continue
    report = json.loads(result.stdout)
    print(f"{path.relative_to(root)}: {report['files_checked']} Rust files, "
          f"{len(report['diagnostics'])} diagnostics", flush=True)
    if report["diagnostics"]:
        print(json.dumps(report["diagnostics"], indent=2), file=sys.stderr)
        failed = True
sys.exit(int(failed))
