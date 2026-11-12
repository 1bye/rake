#!/usr/bin/env python3
"""Require a clean deep report for the workspace and every top-level example."""
import json
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--format-version=1", "--locked"],
    cwd=root, text=True,
))
members = set(metadata["workspace_members"])
member_manifests = {
    Path(package["manifest_path"]).resolve()
    for package in metadata["packages"] if package["id"] in members
}
standalone = []
for manifest in sorted((root / "examples").glob("*/Cargo.toml")):
    if manifest.resolve() in member_manifests:
        print(f"{manifest.parent.relative_to(root)}: covered by workspace --all-targets", flush=True)
    else:
        standalone.append(manifest.parent)

failed = False
for path in [root, *standalone]:
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
