#!/usr/bin/env python3
"""Report an uncompressed upper bound after rust-cache's dependency filtering.

This is read-only: registry git indexes/checkouts are conservatively counted in full;
workspace artifacts, incremental builds and unpacked non-sys sources are omitted.
Archive compression and rust-cache's unused/one-week cleanup reduce this further.
"""

import os
from pathlib import Path
import sys
import tomllib


def size(path):
    if path.is_symlink():
        return 0
    if path.is_file():
        return path.stat().st_size
    if path.is_dir():
        return sum(size(child) for child in path.iterdir())
    return 0


def dependency_targets(target, names):
    names = set(names)
    # Fingerprints record actual library names (gpui-pre builds libgpui, for example).
    for directory in target.rglob(".fingerprint"):
        for package in directory.iterdir():
            if package.name.rsplit("-", 1)[0] not in names or not package.is_dir():
                continue
            for library in package.glob("lib-*"):
                name = library.name.removeprefix("lib-").removesuffix(".json")
                names.update((name, "lib" + name))
    total = 0
    for directory in target.rglob("*"):
        if directory.name not in {"deps", "build", ".fingerprint"} or not directory.is_dir():
            continue
        for entry in directory.iterdir():
            name = entry.name
            if name in names or name.rsplit("-", 1)[0] in names:
                total += size(entry)
    return total


def main():
    packages = tomllib.loads(Path("Cargo.lock").read_text())["package"]
    external = [package for package in packages if "source" in package]
    names = set()
    for package in external:
        name = package["name"]
        names.update((name, name.replace("-", "_"), "lib" + name.replace("-", "_")))
    cargo = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    registry = cargo / "registry"
    archives = {f"{p['name']}-{p['version']}.crate" for p in external}
    sys_packages = {f"{p['name']}-{p['version']}" for p in external if p["name"].endswith("-sys")}
    downloads = sum(size(path) for path in (registry / "cache").glob("*/*") if path.name in archives)
    downloads += sum(size(path) for path in (registry / "index").glob("**/*") if path.is_file() and path.name in names)
    if any(p["source"].startswith("git+") for p in external):
        downloads += size(cargo / "git")
    sys_sources = sum(size(path) for path in (registry / "src").glob("*/*") if path.name in sys_packages)
    target = dependency_targets(Path("target"), names) if "--targets" in sys.argv else 0
    drivers = size(Path.home() / ".dylint_drivers") if "--drivers" in sys.argv else 0
    tools = sum(size(cargo / "bin" / name) for name in ("cargo-dylint", "dylint-link", "wasm-bindgen")) if "--drivers" in sys.argv else 0
    values = dict(downloads=downloads, sys_sources=sys_sources, dependency_targets=target, drivers=drivers, tools=tools)
    values["total"] = sum(values.values())
    report = "\n".join(f"{name}: {value / 2**30:.3f} GiB" for name, value in values.items())
    print(report)
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a") as output:
            output.write("\n### Cache size estimate (uncompressed upper bound)\n\n```text\n" + report + "\n```\n")


if __name__ == "__main__":
    main()
