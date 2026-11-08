#!/usr/bin/env python3
"""CLI contracts after embedded Python removal; internal cases map to native Rust tests.
See crates/rake-cli/PORTING.md. HTTP/native fixtures continue running against the Rust CLI.
"""
import concurrent.futures
import io
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.parse
import urllib.request
import uuid
from rust_contract_tests import rust_contract
ROOT = Path(__file__).resolve().parents[1]
CLI = Path(os.environ.get("CARGO_TARGET_DIR", str(ROOT / "target"))) / "debug/rake"

class PreviewTests(unittest.TestCase):
    def test_selection_keeps_source_and_exact_window_without_a_mutation(self):
        rust_contract(self, ['selection_keeps_source_and_exact_window_without_mutation'], 'bin:rake')

    def test_grab_promotes_text_and_collects_pinned_ancestors_and_descendant_text(self):
        rust_contract(self, ['grab_promotes_text_and_collects_pinned_ancestors_and_descendant_text'], 'bin:rake')

    def test_hover_is_lightweight_and_expired_context_never_uses_another_capture(self):
        rust_contract(self, ['hover_is_lightweight_and_expired_context_never_uses_another_capture'], 'bin:rake')

    def test_annotation_resolution_follows_stable_ids_and_keeps_exact_targets_scoped(self):
        rust_contract(self, ['annotation_resolution_follows_stable_ids_and_keeps_exact_targets_scoped'], 'bin:rake')

    def test_annotation_resolution_pins_pages_and_rejects_other_window_generations(self):
        rust_contract(self, ['annotation_resolution_pins_pages_and_rejects_other_window_generations'], 'bin:rake')

    def test_annotation_resolution_rejects_nonadvancing_pages_and_masks_hidden_targets(self):
        rust_contract(self, ['annotation_resolution_rejects_nonadvancing_pages_and_masks_hidden_targets'], 'bin:rake')

    def test_invalid_annotation_targets_and_expired_references_never_reach_runtime(self):
        rust_contract(self, ['invalid_annotation_targets_and_expired_references_never_reach_runtime'], 'bin:rake')

    def test_click_is_submitted_once_without_a_retry_key(self):
        rust_contract(self, ['click_is_submitted_once_without_retry_key_and_fractional_scroll_keeps_geometry'], 'bin:rake')

    def test_invalid_expired_or_foreign_positions_never_reach_runtime(self):
        rust_contract(self, ['invalid_expired_or_foreign_positions_never_reach_runtime'], 'bin:rake')

    def test_scroll_is_bounded_and_unsupported_input_is_rejected(self):
        rust_contract(self, ['scroll_is_bounded_and_unsupported_input_is_rejected'], 'bin:rake')

    def test_closing_preview_terminates_only_its_helper_and_drops_references(self):
        rust_contract(self, ['closing_capture_reaps_its_helper_and_releases_waiting_viewers', 'preview_state_bounds_references_and_changes_epoch_only_for_window_geometry'], 'bin:rake')

    def test_truncated_and_oversized_capture_frames_fail_explicitly(self):
        rust_contract(self, ['truncated_and_oversized_capture_frames_fail_explicitly'], 'bin:rake')

    def test_fractional_scroll_uses_viewport_identity_and_preserves_phase(self):
        rust_contract(self, ['click_is_submitted_once_without_retry_key_and_fractional_scroll_keeps_geometry'], 'bin:rake')

    def test_video_join_starts_at_keyframe_and_releases_viewer_on_disconnect(self):
        rust_contract(self, ['video_join_starts_at_keyframe_and_releases_viewer_on_disconnect'], 'bin:rake')

    def test_disconnect_during_video_headers_releases_encoder(self):
        rust_contract(self, ['disconnect_during_video_headers_releases_encoder'], 'bin:rake')

    def test_websocket_reads_masked_text_and_rejects_fragments_and_large_frames(self):
        rust_contract(self, ['masked_text_and_control_frames_reject_fragments_unmasked_and_oversized_input'], 'bin:rake')

    def test_preview_flag_rejects_web_and_non_dev_commands(self):
        rust_contract(self, ['parser_preserves_errors_flags_and_doctor_validation_order'], 'test:project_build')

    def test_preview_http_requires_token_and_same_origin(self):
        with tempfile.TemporaryDirectory(prefix="rake-preview-http-", dir="/tmp") as temporary:
            root = Path(temporary)
            from broker_test_server import BrokerServer
            server=BrokerServer(root,native=True,preview=True)
            try:
                url,token=server.url,server.token
                for credential, origin in (("wrong", url), (token, "https://foreign.invalid")):
                    request = urllib.request.Request(url + "/__rake/preview_state", b'{}',
                        {"Content-Type": "application/json", "X-Rake-Token": credential, "Origin": origin})
                    with self.assertRaises(urllib.error.HTTPError) as error:
                        urllib.request.urlopen(request, timeout=3)
                    self.assertEqual(error.exception.code, 403)
                    error.exception.close()
                with self.assertRaises(urllib.error.HTTPError) as error:
                    urllib.request.urlopen(url + '/__rake/preview_control', timeout=3)
                self.assertEqual(error.exception.code, 403)
                error.exception.close()
                request = urllib.request.Request(url + "/__rake/preview_state", b'{}',
                    {"Content-Type": "application/json", "X-Rake-Token": token, "Origin": url})
                with urllib.request.urlopen(request, timeout=3) as response:
                    self.assertIn("Waiting for the native", json.load(response)["error"])
                for asset in ['preview', 'preview.js', 'preview.css', 'preview-stream.js', 'preview-annotations.js']:
                    with urllib.request.urlopen(url + '/__rake/' + asset, timeout=3) as response:
                        self.assertNotIn(token, response.read().decode())
                        self.assertIn("frame-ancestors 'none'", response.headers["Content-Security-Policy"])
                        if asset.endswith('.js'):
                            self.assertIn('text/javascript', response.headers['Content-Type'])
            finally:
                server.close()



if __name__ == "__main__":
    unittest.main()
