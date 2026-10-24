#!/usr/bin/env python3
"""Build the shared example and generate its browser loader with matching wasm-bindgen."""

from pathlib import Path
import argparse
import shutil
import subprocess
import tomllib

project = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--example", choices=["foundation", "routing", "notes"], default="foundation")
parser.add_argument("--hash", action="store_true", help="Build the routing example with fragment URLs")
args = parser.parse_args()

if args.hash and args.example != "routing":
    parser.error("--hash applies to the routing example")

package = f"rake-{args.example}"
subprocess.run([
    "cargo", "build", "--locked", "-p", package, "--lib",
    "--target", "wasm32-unknown-unknown",
    *(["--features", "hash-routing"] if args.hash else []),
], cwd=project, check=True)
lock = tomllib.loads((project / "Cargo.lock").read_text())
version = next(item["version"] for item in lock["package"] if item["name"] == "wasm-bindgen")
binary = shutil.which("wasm-bindgen")

if not binary:
    raise SystemExit(f"Install loader tooling: cargo install wasm-bindgen-cli --version {version} --locked")

actual = subprocess.check_output([binary, "--version"], text=True).strip()

if actual != f"wasm-bindgen {version}":
    raise SystemExit(f"Expected wasm-bindgen {version}, found {actual}")

output = project / "dist" / (args.example + ("-hash" if args.hash else ""))
shutil.copytree(project / "examples" / args.example / "web", output, dirs_exist_ok=True)
subprocess.run([
    binary, "--target", "web", "--out-dir", str(output / "pkg"),
    str(project / "target" / "wasm32-unknown-unknown" / "debug" / f"{package.replace('-', '_')}.wasm"),
], check=True)
print(f"Built {output}; serve with python3 scripts/serve-web.py --example {output.name}")
