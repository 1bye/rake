#!/usr/bin/env python3
"""Check Rust packaging, GPUI pins, optionality, and the stable Web graph."""

import json
from pathlib import Path
import subprocess
import tomllib

root = Path(__file__).resolve().parents[1]
manifest = tomllib.loads((root / "Cargo.toml").read_text())
workspace = manifest["workspace"]["dependencies"]

rust = tomllib.loads((root / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
tools = json.loads((root / "crates/rake-dist/src/flatpak-rust-tools.json").read_text())
for target, components in tools.items():
    for name, component in components.items():
        expected = f"{name}-{rust}-{target}.tar.xz"
        if component["url"].rsplit("/", 1)[-1] != expected:
            raise SystemExit(f"Flatpak {target}/{name} must match Rust {rust}: {component['url']}")


def exact(name):
    value = workspace[name]
    version = value if isinstance(value, str) else value["version"]
    if not version.startswith("="):
        raise SystemExit(f"{name} must have an exact workspace pin, got {version}")
    return version[1:]


def cargo(*arguments):
    return subprocess.check_output(["cargo", *arguments], cwd=root, text=True)


engine = exact("gpui")
if exact("gpui_platform") != engine:
    raise SystemExit("GPUI core and platform pins disagree")
base = exact("gpui-base")
kit = exact("gpui-kit")
if base != kit:
    raise SystemExit("Base and Kit release pins disagree")

metadata = json.loads(cargo("metadata", "--format-version", "1", "--all-features", "--locked"))
packages = metadata["packages"]
expected = {"gpui-pre": engine, "gpui-pre-platform": engine,
            "gpui-base": base, "gpui-kit": kit,
            "gpui-component": kit, "gpui-kit-assets": kit}
for name, version in expected.items():
    resolved = {package["version"] for package in packages if package["name"] == name}
    if resolved != {version}:
        raise SystemExit(f"{name}: expected only {version}, resolved {sorted(resolved)}")

for package in packages:
    if package["name"] not in {"gpui-base", "gpui-kit", "gpui-component", "gpui-kit-assets"}:
        continue
    for dependency in package["dependencies"]:
        name = dependency["name"]
        if name in {"gpui-pre", "gpui-pre-platform", "gpui-pre-web", "gpui-pre-macros"}:
            if dependency["req"] != f"={engine}":
                raise SystemExit(f"{package['name']} pins {name} to {dependency['req']}, expected ={engine}")
        if name == "gpui-base" and dependency["req"].lstrip("^=") != base:
            raise SystemExit(f"{package['name']} expects Base {dependency['req']}, workspace pins {base}")

for name, arguments, forbidden in [
    ("default Rake", [], {"gpui-base", "gpui-kit", "gpui-component", "gpui-kit-assets"}),
    ("stable Web views", ["--features", "experimental-view", "--target", "wasm32-unknown-unknown"],
     {"gpui-kit", "wasm_thread"}),
]:
    tree = cargo("tree", "-p", "rake", "--locked", "--edges", "normal", "--prefix", "none",
                 "--format", "{p}", *arguments)
    names = {line.split(" v", 1)[0] for line in tree.splitlines()}
    if unwanted := names & forbidden:
        raise SystemExit(f"{name} unexpectedly compiles {sorted(unwanted)}")

print(f"Rust {rust} Flatpak pins, GPUI {engine}, Base/Kit {base}; default and stable Web graphs verified")
