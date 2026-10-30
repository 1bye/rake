#!/usr/bin/env python3
"""Covers: native preview clicks, bidirectional scrolling while visible/covered, source and backdrop.
Real: compiled GPUI preview fixture and its Unix development protocol. Doubles: none.
Requires: macOS GUI and cargo build -p rake-preview --features devtools. No browser claims.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from dev_bridge import DevBridge
from inspector import Inspector

ROOT = Path(__file__).resolve().parents[1]


def eventually(check):
    deadline = time.monotonic() + 10
    last = None
    while time.monotonic() < deadline:
        last = check()
        if last:
            return last
        time.sleep(.05)
    raise AssertionError(f"Native preview condition did not become ready: {last}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', nargs='?', type=Path, default=ROOT / 'target/debug/rake-preview')
    parser.add_argument('--occluded', action='store_true', help='Cover the real native window before injecting input')
    args = parser.parse_args()
    if sys.platform != "darwin":
        raise SystemExit("Requires a macOS GUI session; this test does not silently skip")
    with tempfile.TemporaryDirectory(prefix="rake-preview-test-", dir="/tmp") as temporary:
        control = Path(temporary) / "control"
        control.mkdir(mode=0o700)
        with tempfile.TemporaryFile(mode="w+") as log:
            process = subprocess.Popen([str(args.binary)], cwd=ROOT,
                env={**os.environ, "RAKE_DEV_CONTROL": "1", "RAKE_DEV_PREVIEW": "1", "RAKE_CONTROL_DIR": str(control)},
                stdout=log, stderr=log)
            cover = None
            try:
                manifest = eventually(lambda: next((json.loads(p.read_bytes()) for p in control.glob('*.json')), None))
                runtime = manifest["info"]["runtime_id"]
                inspector = Inspector(DevBridge("http://127.0.0.1:0", directory=control), allow_native=True)
                def request(value, command=False):
                    return inspector.handle("inspect", {"request": {"protocol": 1, "runtime_id": runtime,
                        "key": None, "message": {"type": "command" if command else "query", "value": value}}})
                def query(value):
                    reply = request(value)
                    assert "Ok" in reply, reply
                    return reply["Ok"]["value"]
                def tree():
                    return query({"type": "inspect_tree"})["elements"]
                def find(name):
                    return next(e for e in tree() if e.get("semantic_id") == name)
                def point(x, y):
                    window = query({"type": "preview"})[0]
                    return {**{k: window[k] for k in ("window", "native_id", "generation")}, "x": round(x), "y": round(y)}
                def input_at(position, input):
                    reply = request({"type": "preview_input", "point": position, "input": input}, command=True)
                    assert reply == {"Ok": {"type": "acknowledged"}}, reply
                def count():
                    return next(e["text"] for e in tree() if (e.get("text") or "").startswith("Clicks:"))
                eventually(lambda: query({"type": "preview"}))
                if args.occluded:
                    # A separate process is essential: another visible GPUI window in
                    # this process keeps the app's redraw activity alive and hides the bug.
                    cover = subprocess.Popen([str(args.binary)], cwd=ROOT,
                        env={**os.environ, 'RAKE_PREVIEW_TEST_COVER':'1',
                             'RAKE_DEV_CONTROL':'0', 'RAKE_DEV_PREVIEW':'0'}, stdout=log, stderr=log)
                    eventually(lambda: query({'type':'preview'})[0]['occluded'])
                bounds = find("preview-increment")["layout"]
                x, y = bounds["x"] + bounds["width"] / 2, bounds["y"] + bounds["height"] / 2
                for expected, position in enumerate([(x - 1, y), (x, y), (bounds["x"] + 12, y), (x, y)], 1):
                    input_at(point(*position), {"type": "click"})
                    try:
                        eventually(lambda: count() == f"Clicks: {expected}")
                    except AssertionError as error:
                        raise AssertionError(f"Click {expected} at {position}: actual {count()}") from error
                picked = query({"type": "preview_element", "point": point(x, y)})
                assert picked["source"]["file"].endswith("examples/preview/src/main.rs"), picked
                assert picked["source"]["line"] > 0
                assert count() == "Clicks: 4", "selection must not activate the control"
                before = find("preview-row-10")["layout"]["y"]
                bounds = find("preview-scroll")["layout"]
                live = point(bounds["x"] + bounds["width"] / 2, bounds["y"] + 150)
                description = query({'type':'preview'})[0]
                live['geometry'] = [round(description[r][k] * 64) for r,k in
                    [('frame','width'),('frame','height'),('content','x'),('content','y'),('content','width'),('content','height')]]
                input_at(live, {'type':'wheel','dx':0,'dy':16001,'phase':'started'})
                input_at(live, {'type':'wheel','dx':0,'dy':0,'phase':'ended'})
                eventually(lambda: find("preview-row-10")["layout"]["y"] < before)
                after = find('preview-row-10')['layout']['y']
                assert abs(before - after - 250.015625) <= 1, (before, after)
                input_at(live, {'type':'wheel','dx':0,'dy':-8000,'phase':'started'})
                input_at(live, {'type':'wheel','dx':0,'dy':0,'phase':'ended'})
                eventually(lambda: find('preview-row-10')['layout']['y'] > after)
                assert abs(find('preview-row-10')['layout']['y'] - after - 125) <= 1
                if args.occluded:
                    assert query({'type':'preview'})[0]['occluded'], 'cover must remain in place during input'

                bounds = find("preview-blur")["layout"]
                for translucent in (False, True):
                    input_at(point(bounds["x"] + 20, bounds["y"] + 20), {"type": "click"})
                    eventually(lambda: query({"type": "preview"})[0]["translucent"] == translucent)
                stale = point(x, y)
                stale["generation"] = "expired"
                assert "Err" in request({"type": "preview_input", "point": stale, "input": {"type": "click"}}, command=True)
                assert count() == "Clicks: 4"
                print("Native preview passed: repeated atomic clicks, bidirectional fractional scrolling, exact Rust-source picking, blur/opaque toggle and stale rejection" + (" while fully occluded" if args.occluded else ""))
            except BaseException:
                log.seek(0)
                print(log.read(), file=sys.stderr)
                raise
            finally:
                for child in (cover, process):
                    if child is None:
                        continue
                    child.terminate()
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait(timeout=5)


if __name__ == "__main__":
    main()
