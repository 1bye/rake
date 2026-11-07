#!/usr/bin/env python3
"""Run the syntax linter on every example; warnings must be fixed or reasoned."""
import json
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parent.parent
failed = False
for manifest in sorted((root / "examples").glob("*/Cargo.toml")):
    package = manifest.parent.relative_to(root)
    result = subprocess.run(
        ["cargo", "run", "-p", "rake-app-cli", "--locked", "--",
         "lint", str(package), "--json"],
        cwd=root, text=True, capture_output=True, check=False,
    )
    if result.returncode:
        print(result.stderr or result.stdout, file=sys.stderr)
        failed = True
        continue
    report = json.loads(result.stdout)
    print(f"{package}: {report['files_checked']} Rust files, "
          f"{len(report['diagnostics'])} diagnostics")
    if report["diagnostics"]:
        print(json.dumps(report["diagnostics"], indent=2), file=sys.stderr)
        failed = True
sys.exit(1 if failed else 0)
