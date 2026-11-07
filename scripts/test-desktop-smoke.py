#!/usr/bin/env python3
"""Covers: smoke runner rejects missing frames, crashes, panics and hangs.
Real: child processes, exit status, logs and timeout cleanup. Doubles: app output.
Requires: Python only; this suite makes no GPU claims.
"""
from pathlib import Path
import runpy
import sys
import tempfile
import unittest

smoke = runpy.run_path(str(Path(__file__).with_name("desktop-smoke.py")))["smoke"]


class SmokeTests(unittest.TestCase):
    def run_app(self, source, timeout=5):
        with tempfile.TemporaryDirectory() as directory:
            smoke([sys.executable, "-c", source], Path(directory) / "smoke.log", timeout)

    def test_frame_and_clean_exit(self):
        self.run_app('print("RAKE_DESKTOP_SMOKE_FRAME")')

    def test_missing_frame(self):
        with self.assertRaisesRegex(AssertionError, "No rendered-frame"):
            self.run_app('print("window created")')

    def test_nonzero_even_after_frame(self):
        with self.assertRaisesRegex(AssertionError, "exited with 1"):
            self.run_app('print("RAKE_DESKTOP_SMOKE_FRAME"); raise SystemExit(1)')

    def test_background_panic_even_with_clean_exit(self):
        with self.assertRaisesRegex(AssertionError, "panicked at"):
            self.run_app('print("RAKE_DESKTOP_SMOKE_FRAME"); print("thread panicked at render")')

    def test_timeout(self):
        with self.assertRaisesRegex(AssertionError, "exceeded"):
            self.run_app("import time; time.sleep(30)", timeout=0.1)


if __name__ == "__main__":
    unittest.main()
