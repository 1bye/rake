#!/usr/bin/env python3
"""Explicit native/protocol tray probes, never destructive desktop restarts."""
import argparse
from pathlib import Path
import shutil
import subprocess
import sys

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("backend", choices=("appkit", "sni", "gpui"))
args = parser.parse_args()
if args.backend == "appkit":
    if sys.platform != "darwin":
        parser.error("AppKit probe requires macOS and a graphical session")
    command = ["cargo", "run", "-p", "rake-tray", "--features", "native-tests", "--example", "native-probe", "--locked"]
elif args.backend == "gpui":
    command = ["cargo", "run", "-p", "rake-tray-example", "--locked", "--", "--probe"]
else:
    daemon = shutil.which("dbus-run-session")
    if not daemon:
        parser.error("SNI protocol probe requires dbus-run-session/dbus-daemon")
    command = [daemon, "--config-file", str(root / "crates/rake-tray/fixtures/session.conf"), "--", "cargo", "run", "-p", "rake-tray", "--features", "sni-tests", "--example", "sni-probe", "--locked"]
try:
    result = subprocess.run(command, cwd=root, check=True, timeout=300, text=True, stdout=subprocess.PIPE)
    print(result.stdout, end="")
    marker = {"appkit": "native tray probe passed", "sni": "SNI protocol probe passed", "gpui": "GPUI tray probe passed"}[args.backend]
    if marker not in result.stdout:
        sys.exit("tray probe exited before its success marker")
except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
    sys.exit(f"tray probe failed: {error}")
