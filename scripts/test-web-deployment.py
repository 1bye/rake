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

class DeploymentTests(unittest.TestCase):
    def test_default_icon_is_shared_with_native_apps_and_installation_is_idempotent(self):
        rust_contract(self, ['default_icon_is_shared_and_installation_is_idempotent'], 'bin:rake')

    def test_authored_favicons_including_explicit_empty_icons_are_preserved(self):
        rust_contract(self, ['authored_favicons_including_empty_icons_are_preserved'], 'bin:rake')

    def test_comments_and_script_strings_do_not_count_as_favicon_links(self):
        rust_contract(self, ['comments_and_script_strings_do_not_count_as_favicon_links'], 'bin:rake')

    def test_implicit_head_is_supported(self):
        rust_contract(self, ['implicit_head_is_supported'], 'bin:rake')

    def test_generated_asset_does_not_overwrite_an_application_file(self):
        rust_contract(self, ['generated_icon_does_not_overwrite_an_application_file'], 'bin:rake')

    def test_offline_builds_pin_assets_and_keep_router_root_separate(self):
        rust_contract(self, ['offline_builds_pin_assets_and_keep_router_root_separate'], 'bin:rake')

    def test_online_bundles_have_version_checks_without_changing_asset_layout(self):
        rust_contract(self, ['online_bundles_have_version_checks_without_changing_asset_layout'], 'bin:rake')

    def test_configuration_rejects_nonboolean_offline(self):
        rust_contract(self, ['web_metadata_rejects_nonboolean_offline_and_accepts_native_sections'], 'bin:rake')

    def test_shared_application_metadata_accepts_native_sections_on_web(self):
        rust_contract(self, ['web_metadata_rejects_nonboolean_offline_and_accepts_native_sections'], 'bin:rake')

    def test_generated_names_cannot_collide_with_inputs(self):
        rust_contract(self, ['generated_names_and_development_metadata_cannot_collide_with_inputs'], 'bin:rake')



if __name__ == "__main__":
    unittest.main()
