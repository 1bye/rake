#!/usr/bin/env python3
"""Measure warmed native preview scroll draws; requires macOS and a devtools build."""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

from native_test_client import exchange


def eventually(operation):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        result = operation()
        if result:
            return result
        time.sleep(.05)
    raise AssertionError("Native preview did not become ready")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="rake-scroll-", dir="/tmp") as temporary:
        control = Path(temporary)
        control.chmod(0o700)
        with tempfile.TemporaryFile(mode="w+") as log:
            process = subprocess.Popen([str(args.binary.resolve())],
                env={**os.environ, "RAKE_DEV_CONTROL": "1", "RAKE_DEV_PREVIEW": "1",
                     "RAKE_CONTROL_DIR": str(control)}, stdout=log, stderr=log)
            try:
                def discovery():
                    for path in control.glob("*.json"):
                        try:
                            return json.loads(path.read_bytes())
                        except (FileNotFoundError, json.JSONDecodeError):
                            continue
                instance = eventually(discovery)
                query = lambda value: exchange(instance, "query", value)
                command = lambda value: exchange(instance, "command", value)
                window = eventually(lambda: query({"type": "preview"}))[0]
                elements = query({"type": "inspect_tree"})["elements"]
                bounds = next(e["layout"] for e in elements if e.get("semantic_id") == "preview-scroll")
                point = {**{k: window[k] for k in ("window", "native_id", "generation")},
                         "x": round(bounds["x"] + bounds["width"] / 2), "y": round(bounds["y"] + 150),
                         "geometry": [round(window[r][k] * 64) for r, k in
                             [("frame", "width"), ("frame", "height"), ("content", "x"),
                              ("content", "y"), ("content", "width"), ("content", "height")]]}
                def row_y():
                    return next(e["layout"]["y"] for e in query({"type": "inspect_tree"})["elements"]
                                if e.get("semantic_id") == "preview-row-10")
                before = row_y()
                def scroll():
                    command({"type": "preview_input", "point": point,
                             "input": {"type": "wheel", "dx": 0, "dy": 64, "phase": "moved"}})
                    time.sleep(.02)
                for _ in range(8):
                    scroll()
                command({"type": "record_frames", "enabled": True, "lease_ms": 5000})
                try:
                    for _ in range(25):
                        scroll()
                finally:
                    command({"type": "record_frames", "enabled": False, "lease_ms": 0})
                times = sorted(f["draw_us"] / 1000 for f in query({"type": "frames", "after": 0})["frames"])
                assert len(times) >= 15, f"Insufficient native draws: {len(times)}"
                assert row_y() < before, "Scroll input did not move content"
                assert all(t > 0 for t in times), times
                print(json.dumps({"draws": len(times), "median_draw_ms": statistics.median(times),
                    "p95_draw_ms": times[min(len(times) - 1, int(len(times) * .95))], "max_draw_ms": max(times)}))
            except BaseException:
                log.seek(0)
                print(log.read())
                raise
            finally:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


if __name__ == "__main__":
    main()
