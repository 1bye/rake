#!/usr/bin/env python3
"""Build an example through the Rust CLI with matching wasm-bindgen tooling."""
import argparse
import os
from pathlib import Path
import subprocess

project = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--example", choices=["foundation", "routing", "notes", "view", "files"], default="foundation")
parser.add_argument("--hash", action="store_true", help="Build the routing example with fragment URLs")
args = parser.parse_args()
if args.hash and args.example != "routing":
    parser.error("--hash applies to the routing example")

cli = Path(os.environ.get("RAKE_CLI", str(Path(os.environ.get("CARGO_TARGET_DIR", str(project / "target"))) / "debug/rake")))
output = project / "dist" / (args.example + ("-hash" if args.hash else ""))
subprocess.run([str(cli), "build", "--target", "web", "--manifest-path",
    str(project / "examples" / args.example / "Cargo.toml"), "--output", str(output),
    *(["--features", "hash-routing"] if args.hash else [])], cwd=project, check=True)
print(f"Built {output}; serve with python3 -m http.server --directory {output} 8080, or use rake dev for inspection")
