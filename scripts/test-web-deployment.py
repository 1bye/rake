#!/usr/bin/env python3
"""Real deployment generation and archive inputs; no browser/GPU claim."""

import json
from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest

from bundle import BundleConfig, install_favicon, web_deployment


class FaviconTests(unittest.TestCase):
    def test_default_icon_is_shared_with_native_apps_and_installation_is_idempotent(self):
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            document = '<!doctype html><html><head><title>App</title></head><body>app</body></html>'
            (site / "index.html").write_text(document)
            install_favicon(site)

            link = '<link rel="icon" type="image/svg+xml" href="/assets/rake-icon.svg">'
            installed = (site / "index.html").read_text()
            self.assertEqual(installed.replace(link, ""), document)
            source = Path(__file__).resolve().parents[1] / "crates/rake-build/assets/default-icon.svg"
            self.assertEqual((site / "assets/rake-icon.svg").read_bytes(), source.read_bytes())
            install_favicon(site)
            self.assertEqual((site / "index.html").read_text(), installed)
            # The example build script recopies its host HTML on each build.
            (site / "index.html").write_text(document)
            install_favicon(site)
            self.assertEqual((site / "index.html").read_text(), installed)

    def test_authored_favicons_including_explicit_empty_icons_are_preserved(self):
        for link in [
            '<link rel="icon" href="./custom.svg">',
            "<LINK REL='shortcut ICON' HREF='custom.ico'>",
            '<link rel=icon href="data:,">',
            '<link href="custom.png" rel="icon alternate"/>',
        ]:
            with self.subTest(link=link), tempfile.TemporaryDirectory() as directory:
                site = Path(directory)
                document = f'<html><head>{link}</head><body></body></html>'
                (site / "index.html").write_text(document)
                install_favicon(site)
                self.assertEqual((site / "index.html").read_text(), document)
                self.assertFalse((site / "assets").exists())

    def test_comments_and_script_strings_do_not_count_as_favicon_links(self):
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            document = '''<!-- <head><link rel="icon" href="comment.svg"> -->
<html>
<head><script>const example = '<link rel="icon" href="script.svg">';</script></head></html>'''
            (site / "index.html").write_text(document)
            install_favicon(site)
            self.assertIn('<head><link rel="icon" type="image/svg+xml"', (site / "index.html").read_text())
            self.assertTrue((site / "assets/rake-icon.svg").is_file())

    def test_implicit_head_is_supported(self):
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            (site / "index.html").write_text('<html lang="en"><meta charset="utf-8"><body>App</body></html>')
            install_favicon(site)
            self.assertIn('<html lang="en"><link rel="icon"', (site / "index.html").read_text())

    def test_generated_asset_does_not_overwrite_an_application_file(self):
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            document = '<html><head></head></html>'
            (site / "index.html").write_text(document)
            (site / "assets").mkdir()
            (site / "assets/rake-icon.svg").write_text("app-owned")
            with self.assertRaises(FileExistsError):
                install_favicon(site)
            self.assertEqual((site / "assets/rake-icon.svg").read_text(), "app-owned")
            self.assertEqual((site / "index.html").read_text(), document)


class DeploymentTests(unittest.TestCase):
    def generate(self, root, *, offline=True, version="1.0.0", payload="old", base="/tools/"):
        root.mkdir()
        (root / "pkg").mkdir()
        (root / "pkg/app.js").write_text(payload)
        (root / "pkg/app_bg.wasm").write_bytes(b"\0asm" + payload.encode())
        (root / "index.html").write_text('<html><head></head><script type="module">import init from "/pkg/app.js";init();</script></html>')
        install_favicon(root)
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
            self.assertIn('href="' + prefix + 'assets/rake-icon.svg"', index)
            self.assertTrue((root / "old" / prefix.removeprefix("/tools/") / "assets/rake-icon.svg").is_file())
            self.assertIn(f'src="{prefix}rake-offline.js"', index)
            self.assertIn('name="rake-deployment"', index)
            self.assertEqual((root / "old" / prefix.removeprefix("/tools/") / "pkg/app.js").read_text(), "old")
            self.assertFalse((root / "old/pkg").exists())
            script = (root / "old/rake-service-worker.js").read_text()
            self.assertIn(prefix + "pkg/app_bg.wasm", script)
            self.assertIn(prefix + "assets/rake-icon.svg", script)
            self.assertNotIn("self.skipWaiting(", script)
            self.assertNotIn("clients.claim(", script)

    def test_online_bundles_have_version_checks_without_changing_asset_layout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "site"
            metadata = self.generate(root, offline=False, base="/")
            self.assertFalse(metadata["offline"])
            self.assertTrue((root / "pkg/app.js").is_file())
            self.assertTrue((root / "rake-version.json").is_file())
            self.assertIn('href="/assets/rake-icon.svg"', (root / "index.html").read_text())
            self.assertFalse((root / "rake-service-worker.js").exists())

    def test_configuration_rejects_nonboolean_offline(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "boolean"):
                BundleConfig({"name": "a", "version": "1.0.0", "metadata": {"rake": {"bundle": {"web": {"offline": "yes"}}}}}, Path(directory))

    def test_shared_application_metadata_accepts_native_sections_on_web(self):
        with tempfile.TemporaryDirectory() as directory:
            package = {"name": "example", "version": "1.0.0", "metadata": {"rake": {"bundle": {
                "schema_version": 1, "web": {"offline": True}, "windows": {"manifest": "windows.manifest.xml"},
                "macos": {"formats": ["dmg"]}, "linux": {"formats": ["deb"]}, "resources": [], "updates": {"enabled": False},
            }}}}
            self.assertTrue(BundleConfig(package, Path(directory)).offline)

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
