"""Run the Rust replacements for deleted helper-internal tests, requiring actual execution.

The original script test names and their Rust mappings are retained in
ported-contract-tests.json and PORTING.md. No zero-test or skipped result counts
as passing a translated contract.
"""
import functools
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]


@functools.cache
def _run(target, name):
    kind, binary = target.split(":", 1)
    result = subprocess.run(
        ["cargo", "test", "-p", "rake-app-cli", "--locked", "--" + kind,
         binary, name, "--", "--nocapture"],
        cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )
    passed = re.search(r"^test (?:\S*::)?" + re.escape(name) + r" \.\.\. ok$", result.stdout, re.M)
    return result.returncode == 0 and bool(passed), result.stdout


def rust_contract(case, names, target="bin:rake"):
    for name in names:
        passed, output = _run(target, name)
        case.assertTrue(passed, f"Rust replacement {target}/{name} did not pass:\n{output[-12000:]}")
