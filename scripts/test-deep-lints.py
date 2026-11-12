#!/usr/bin/env python3
"""Compile real GPUI UI fixtures and compare every deep diagnostic to its marker."""
import collections
import json
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
library = root / "crates/rake-cli/lints/rake-gpui"
fixture = library / "ui"
expected = collections.Counter()
for source in sorted((fixture / "src").glob("*.rs")):
    for line, text in enumerate(source.read_text().splitlines(), 1):
        if "//~ " in text:
            expected[(source.name, line, text.split("//~ ", 1)[1].strip())] += 1
result = subprocess.run(
    ["cargo", "dylint", "--path", str(library), "--manifest-path",
     str(fixture / "Cargo.toml"), "--", "--lib", "--locked",
     "--message-format=json"],
    cwd=root, text=True, stdout=subprocess.PIPE, check=False,
)
actual = collections.Counter()
built = False
for line in result.stdout.splitlines():
    if not line.startswith("{"):
        continue
    event = json.loads(line)
    if event.get("reason") == "build-finished":
        built = event.get("success", False)
    if event.get("reason") != "compiler-message":
        continue
    diagnostic = event["message"]
    code = (diagnostic.get("code") or {}).get("code", "")
    if diagnostic["level"] == "error":
        print(diagnostic.get("rendered") or diagnostic["message"])
    if code.startswith("gpui_"):
        for span in diagnostic["spans"]:
            if span["is_primary"]:
                actual[(Path(span["file_name"]).name, span["line_start"], code)] += 1
assert result.returncode == 0 and built, "Dylint UI compilation failed"
assert actual == expected, f"Missing: {expected - actual}\nUnexpected: {actual - expected}"
print(f"Deep UI tests passed: {sum(expected.values())} expected diagnostics; no unexpected diagnostics")
