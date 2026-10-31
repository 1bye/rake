#!/usr/bin/env python3
"""Real deployment generation and archive inputs; no browser/GPU claim."""

import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest

from bundle import BundleConfig, web_deployment


class DeploymentTests(unittest.TestCase):
    def generate(self, root, *, offline=True, version="1.0.0", payload="old", base="/tools/"):
        root.mkdir()
        (root / "pkg").mkdir()
        (root / "pkg/app.js").write_text(payload)
        (root / "pkg/app_bg.wasm").write_bytes(b"\0asm" + payload.encode())
        (root / "index.html").write_text('<html><head></head><script type="module">import init from "/pkg/app.js";init();</script></html>')
        package = {"name": "example", "version": version, "metadata": {"rake": {"bundle": {"identifier": "dev.example.tools", "web": {"base_path": base, "offline": offline}}}}}
        project = SimpleNamespace(package=package, bundle=BundleConfig(package, root), args=SimpleNamespace(base_path=None, routing=None))
        return web_deployment(project, root)

    def test_offline_builds_pin_assets_and_keep_router_root_separate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            old = self.generate(root / "old")
            same = self.generate(root / "same")
            new = self.generate(root / "new", payload="new")
            self.assertEqual(old["build"], same["build"])
            self.assertNotEqual(old["build"], new["build"])
            manifest = json.loads((root / "old/rake-version.json").read_text())
            self.assertEqual(manifest["base_path"], "/tools/")
            self.assertEqual(manifest["build"], old["build"])
            index = (root / "old/index.html").read_text()
            prefix = f'/tools/rake-builds/{old["build"]}/'
            self.assertIn(f'<base href="{prefix}">', index)
            self.assertIn(prefix + "pkg/app.js", index)
            self.assertIn(f'src="{prefix}rake-offline.js"', index)
            self.assertIn('name="rake-deployment"', index)
            self.assertEqual((root / "old" / prefix.removeprefix("/tools/") / "pkg/app.js").read_text(), "old")
            self.assertFalse((root / "old/pkg").exists())
            script = (root / "old/rake-service-worker.js").read_text()
            self.assertIn(prefix + "pkg/app_bg.wasm", script)
            self.assertNotIn("self.skipWaiting(", script)
            self.assertNotIn("clients.claim(", script)

    def test_online_bundles_have_version_checks_without_changing_asset_layout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "site"
            metadata = self.generate(root, offline=False, base="/")
            self.assertFalse(metadata["offline"])
            self.assertTrue((root / "pkg/app.js").is_file())
            self.assertTrue((root / "rake-version.json").is_file())
            self.assertFalse((root / "rake-service-worker.js").exists())

    def test_configuration_rejects_nonboolean_offline(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "boolean"):
                BundleConfig({"name": "a", "version": "1.0.0", "metadata": {"rake": {"bundle": {"web": {"offline": "yes"}}}}}, Path(directory))

    def test_generated_names_cannot_collide_with_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "site"
            self.generate(root)
            package = {"name": "example", "version": "1.0.0"}
            project = SimpleNamespace(package=package, bundle=BundleConfig(package, root), args=SimpleNamespace(base_path=None, routing=None))
            with self.assertRaisesRegex(ValueError, "reserved|existing deployment"):
                web_deployment(project, root)


if __name__ == "__main__":
    unittest.main()
