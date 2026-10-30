#!/usr/bin/env python3
"""Covers: relocatable native bundles, signing/dylibs, Web deployment, assets and rollback.
Real: Cargo/Rust, wasm-bindgen, native platform tools, archives and loopback HTTP.
Doubles: tiny application fixtures; Linux layout uses a platform selector double on macOS.
Requires: Python 3.11+, Rust with wasm32 target, matching wasm-bindgen; macOS signing/clang tools on macOS.
"""

import base64
import hashlib
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import tempfile
import threading
import tomllib
import unittest
import urllib.error
import urllib.request
from unittest.mock import patch
import zipfile

from bundle import BundleConfig, base_path, copy_tree, native

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "target/debug/rake"


def run(command, **options):
    result = subprocess.run(command, text=True, capture_output=True, **options)
    if result.returncode:
        raise AssertionError(f"{command[0]} failed:\n{result.stderr[-8000:]}\n{result.stdout[-2000:]}")
    return result.stdout


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="rake-bundle-", dir="/tmp")
        self.root = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def fixture(self, web=False, source=None):
        root = self.root / "project"
        (root / "src").mkdir(parents=True)
        (root / "assets/nested").mkdir(parents=True)
        (root / "assets/nested/message.txt").write_text("owned packaged asset")
        manifest = '[package]\nname="bundle-fixture"\nversion="1.2.3-beta.1"\nedition="2024"\n'
        if web:
            version = next(item["version"] for item in tomllib.loads((ROOT / "Cargo.lock").read_text())["package"] if item["name"] == "wasm-bindgen")
            manifest += f'[lib]\ncrate-type=["cdylib"]\n[dependencies]\nwasm-bindgen="={version}"\n'
            (root / "src/lib.rs").write_text('use wasm_bindgen::prelude::*; #[wasm_bindgen] pub fn message()->String{"fixture".into()}')
            (root / "web").mkdir()
            (root / "web/index.html").write_text('<!doctype html><html><head><title>Fixture</title></head><body><script type="module" src="./loader.js"></script></body></html>')
            (root / "web/loader.js").write_text("import init from '/pkg/bundle_fixture.js'; await init(); fetch('/assets/nested/message.txt');")
        else:
            if sys.platform == "darwin":
                (root / "icon.png").write_bytes(base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQSDjwHwAEJAJA6qdhxgAAAABJRU5ErkJggg=="))
            (root / "src/main.rs").write_text(source or '''fn main(){let exe=std::env::current_exe().unwrap();
let prefix=exe.parent().unwrap().parent().unwrap();
let assets=if cfg!(target_os="macos"){prefix.join("Resources/assets")}else{prefix.join("share/com.example.fixture/assets")};
println!("{}",std::fs::read_to_string(assets.join("nested/message.txt")).unwrap());}''')
        manifest += '[package.metadata.rake.bundle]\nname="Bundle Fixture"\nidentifier="com.example.fixture"\nassets="assets"\nbuild_number="42.5"\n'
        if not web and sys.platform == "darwin":
            manifest += 'icon="icon.png"\n'
        (root / "Cargo.toml").write_text(manifest)
        run(["cargo", "generate-lockfile", "--offline"], cwd=root)
        return root

    def command(self, root, *arguments):
        return [str(CLI), "bundle", "--manifest-path", str(root / "Cargo.toml"), "--output", str(self.root / "output"), "--json", *arguments]

    def test_real_native_bundle_is_signed_relocatable_and_preserves_previous_output_on_failure(self):
        root = self.fixture()
        command = self.command(root)
        result = json.loads(run(command))
        manifest = json.loads(Path(result["manifest"]).read_text())
        self.assertEqual(manifest["profile"], "release")
        self.assertEqual(manifest["application_id"], "com.example.fixture")
        self.assertEqual(result["sha256"], hashlib.sha256(Path(result["archive"]).read_bytes()).hexdigest())
        self.assertFalse(manifest["notarized"])
        archive_before = Path(result["archive"]).read_bytes()
        if sys.platform == "darwin":
            app = Path(result["artifact"])
            info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
            self.assertEqual(info["CFBundleShortVersionString"], "1.2.3")
            self.assertEqual(info["CFBundleVersion"], "42.5")
            self.assertEqual(info["CFBundleIdentifier"], "com.example.fixture")
            self.assertEqual(info["CFBundleIconFile"], "rake-icon.icns")
            self.assertEqual((app / "Contents/Resources/rake-icon.icns").read_bytes()[:4], b"icns")
            self.assertEqual(manifest["signing"], "ad_hoc")
            run(["codesign", "--verify", "--deep", "--strict", str(app)])
            relocated = self.root / "relocated"
            relocated.mkdir()
            run(["ditto", "-x", "-k", result["archive"], str(relocated)])
            executable = relocated / app.name / "Contents/MacOS/bundle-fixture"
        else:
            executable = Path(result["artifact"])
        self.assertEqual(run([str(executable)], cwd="/").strip(), "owned packaged asset")
        if sys.platform == "darwin":
            entitlements = root / "invalid.plist"
            entitlements.write_text("invalid entitlements")
            failed_signing = subprocess.run([*command, "--entitlements", str(entitlements)], capture_output=True)
            self.assertNotEqual(failed_signing.returncode, 0)
            self.assertEqual(Path(result["archive"]).read_bytes(), archive_before)
        # A previous Python bundle only has this exact legacy ownership marker.
        (Path(result["manifest"]).parent / ".rake-distribution").unlink()
        migrated = json.loads(run(command))
        self.assertTrue((Path(migrated["manifest"]).parent / ".rake-distribution").is_file())
        archive_before = Path(result["archive"]).read_bytes()
        (root / "src/main.rs").write_text('fn main(){does_not_compile}')
        failure = subprocess.run(command, capture_output=True, text=True)
        self.assertNotEqual(failure.returncode, 0)
        self.assertEqual(Path(result["archive"]).read_bytes(), archive_before)
        unrelated = self.root / "unrelated"
        unrelated.mkdir(); (unrelated / "keep").write_text("user data")
        refused = command.copy(); refused[refused.index("--output") + 1] = str(unrelated)
        self.assertNotEqual(subprocess.run(refused, capture_output=True).returncode, 0)
        self.assertEqual((unrelated / "keep").read_text(), "user data")

    @unittest.skipUnless(sys.platform == "darwin", "real Mach-O/codesign boundary requires macOS")
    def test_real_dylib_dependency_is_relocated_and_remains_loadable_without_the_original(self):
        root = self.fixture(source='#[link(name="answer")] unsafe extern "C"{fn answer()->i32;} fn main(){println!("{}",unsafe{answer()});}')
        libraries = root / "libraries"
        libraries.mkdir()
        source = libraries / "answer.c"
        source.write_text('int answer(void){return 42;}')
        library = libraries / "libanswer.dylib"
        run(["clang", "-dynamiclib", str(source), "-install_name", str(library), "-o", str(library)])
        env = {**os.environ, "RUSTFLAGS": f"-Lnative={libraries} -Clink-arg=-Wl,-headerpad_max_install_names"}
        result = json.loads(run(self.command(root, "--debug"), env=env))
        app = Path(result["artifact"])
        embedded = app / "Contents/Frameworks/libanswer.dylib"
        self.assertTrue(embedded.is_file())
        shutil.rmtree(libraries)
        self.assertEqual(run([str(app / "Contents/MacOS/bundle-fixture")], cwd="/").strip(), "42")
        run(["codesign", "--verify", "--deep", "--strict", str(app)])

    def test_real_web_bundle_has_deployment_metadata_bindings_assets_and_no_development_bridge(self):
        root = self.fixture(web=True)
        result = json.loads(run(self.command(root, "--target", "web", "--base-path", "/notes/v1/", "--routing", "hash")))
        site = Path(result["artifact"])
        deployment = json.loads((site / "rake-deployment.json").read_text())
        self.assertEqual(deployment["routing"], "hash")
        self.assertEqual(deployment["base_path"], "/notes/v1/")
        self.assertFalse(deployment["development_control"])
        self.assertIn('/notes/v1/pkg/bundle_fixture.js', (site / "loader.js").read_text())
        self.assertIn('href="/notes/v1/"', (site / "index.html").read_text())
        self.assertEqual((site / "assets/nested/message.txt").read_text(), "owned packaged asset")
        self.assertEqual((site / "pkg/bundle_fixture_bg.wasm").read_bytes()[:4], b'\0asm')
        with zipfile.ZipFile(result["archive"]) as archive:
            self.assertIn("index.html", archive.namelist())
            self.assertIn("pkg/bundle_fixture_bg.wasm", archive.namelist())
            self.assertNotIn("site/index.html", archive.namelist())
        for path in [site / "index.html", site / "loader.js"]:
            self.assertNotIn("rake-dev-bridge", path.read_text())
            self.assertNotIn("/__rake/", path.read_text())
        public = self.root / "public"
        shutil.copytree(site, public / "notes/v1")
        class Static(SimpleHTTPRequestHandler):
            def log_message(self, *_args): pass
        with ThreadingHTTPServer(("127.0.0.1", 0), partial(Static, directory=str(public))) as server:
            thread = threading.Thread(target=server.serve_forever)
            thread.start()
            try:
                origin = f"http://127.0.0.1:{server.server_port}"
                with urllib.request.urlopen(origin + "/notes/v1/pkg/bundle_fixture_bg.wasm") as response:
                    self.assertEqual(response.headers.get_content_type(), "application/wasm")
                    self.assertEqual(response.read()[:4], b'\0asm')
                with urllib.request.urlopen(origin + "/notes/v1/assets/nested/message.txt") as response:
                    self.assertEqual(response.read(), b"owned packaged asset")
                with self.assertRaises(urllib.error.HTTPError) as error:
                    urllib.request.urlopen(origin + "/__rake/bootstrap.js")
                self.assertEqual(error.exception.code, 404)
                error.exception.close()
            finally:
                server.shutdown(); thread.join(timeout=5)
        before = Path(result["archive"]).read_bytes()
        (root / "web/index.html").write_text('<html><meta name="rake-dev-bridge" content="secret"></html>')
        self.assertNotEqual(subprocess.run(self.command(root, "--target", "web"), capture_output=True).returncode, 0)
        self.assertEqual(Path(result["archive"]).read_bytes(), before)

    def test_explicit_inputs_reject_links_special_files_collisions_and_invalid_metadata(self):
        defaults = BundleConfig({"name": "fixture", "version": "1.0.0", "metadata": None}, self.root)
        self.assertEqual(defaults.identifier, "dev.rake.fixture")
        self.assertIsNone(defaults.assets)
        source = self.root / "assets"
        source.mkdir()
        (source / "link").symlink_to("/etc/passwd")
        with self.assertRaises(ValueError): copy_tree(source, self.root / "copy")
        (source / "link").unlink()
        os.mkfifo(source / "pipe")
        with self.assertRaises(ValueError): copy_tree(source, self.root / "copy2")
        (source / "pipe").unlink()
        (source / "file").write_text("one")
        copy_tree(source, self.root / "copy3")
        with self.assertRaises(ValueError): copy_tree(source, self.root / "copy3")
        for invalid in ["", "notes/", "/notes", "//notes/", "/a/../", "/a/./", "/a//b/", "/a%2fb/", "/a?b/", "/漢/"]:
            with self.assertRaises(ValueError): base_path(invalid)
        for config in [{"identifier": "not an id"}, {"name": "../escape"}, {"assets": "../"}, {"typo": True}, {"web": {"routing": "wrong"}}]:
            with self.assertRaises((ValueError, FileNotFoundError)):
                BundleConfig({"name": "fixture", "version": "1.0.0", "metadata": {"rake": {"bundle": config}}}, self.root)

    def test_cli_rejects_invalid_target_options_before_building(self):
        for arguments in [["--target", "web", "--base-path", ""], ["--target", "desktop", "--base-path", "/notes/"], ["--target", "web", "--sign-identity", "-"]]:
            result = subprocess.run([str(CLI), "bundle", *arguments, "--json"], text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("error", json.loads(result.stderr))

    def test_linux_resource_layout_uses_the_same_metadata_without_claiming_host_execution(self):
        from types import SimpleNamespace
        source = self.root / "program"
        source.write_bytes(b"fixture executable")
        assets = self.root / "assets"
        assets.mkdir(); (assets / "data").write_text("asset")
        config = BundleConfig({"name": "fixture", "version": "1.0.0", "metadata": {"rake": {"bundle": {"assets": "assets"}}}}, self.root)
        project = SimpleNamespace(bundle=config, target={"name": "fixture"})
        with patch("bundle.sys.platform", "linux"):
            binary = native(project, source, self.root / "linux")
        self.assertEqual(binary.relative_to(self.root / "linux").as_posix(), "bin/fixture")
        self.assertEqual((self.root / "linux/share/dev.rake.fixture/assets/data").read_text(), "asset")


if __name__ == "__main__":
    unittest.main()
