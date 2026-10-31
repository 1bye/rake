#!/usr/bin/env python3
"""Native controller probes against an existing signed Sparkle 2.10.1 appcast.

Runs outside the macOS sandbox with an active graphical login. Does not install
an update, change signing profiles, or modify the supplied feed/public key.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[1]

def run(argv, **kwargs):
    return subprocess.check_output([str(value) for value in argv], text=True, timeout=600, **kwargs)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--appcast', type=Path, required=True)
    parser.add_argument('--public-key', type=Path, required=True)
    args = parser.parse_args()
    assert os.uname().sysname == 'Darwin'
    data = args.appcast.read_bytes()
    requests = []
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            if self.path != '/appcast.xml':
                self.send_error(404)
                return
            body = self.server.feed
            self.send_response(200)
            self.send_header('Content-Type', 'application/xml')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    server.feed = data
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix='rake-sparkle-state-', dir='/private/tmp') as temporary:
            root = Path(temporary)
            (root / 'src').mkdir()
            (root / 'src/main.rs').write_text((ROOT / 'crates/rake-updates/examples/sparkle-state.rs').read_text())
            (root / 'build.rs').write_text('fn main(){rake_build::build().unwrap();}')
            (root / 'sparkle.pub').write_bytes(args.public_key.read_bytes())
            (root / 'feed-url.txt').write_text(f'http://127.0.0.1:{server.server_port}/appcast.xml')
            (root / 'Info.plist').write_text('<?xml version="1.0"?><plist version="1.0"><dict><key>NSAppTransportSecurity</key><dict><key>NSAllowsArbitraryLoads</key><true/></dict></dict></plist>')
            for version in ('1.0.0', '1.0.1'):
                (root / 'Cargo.toml').write_text(f'''[workspace]
[package]
name="sparkle-controller-probe"
version="{version}"
edition="2024"
[dependencies]
rake-updates={{path={json.dumps(str(ROOT / 'crates/rake-updates'))},features=["native-tests","sparkle"]}}
rake-resources={{path={json.dumps(str(ROOT / 'crates/rake-resources'))}}}
futures="0.3"
[build-dependencies]
rake-build={{path={json.dumps(str(ROOT / 'crates/rake-build'))}}}
[package.metadata.rake.bundle]
schema_version=1
name="Rake Controller Probe"
identifier="dev.rake.controller-probe"
resources=[{{from="feed-url.txt",to="feed-url.txt"}}]
[package.metadata.rake.bundle.macos]
info_plist="Info.plist"
[package.metadata.rake.bundle.updates]
enabled=true
base_url="https://example.invalid"
[package.metadata.rake.bundle.updates.public_keys]
sparkle="sparkle.pub"
''')
                env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / 'target'))
                run(['cargo', 'generate-lockfile', '--offline', '--manifest-path', root / 'Cargo.toml'], env=env)
                output = root / ('out-' + version)
                result = json.loads(run([args.cli.resolve(), 'bundle', '--manifest-path', root / 'Cargo.toml', '--format', 'app', '--debug', '--output', output, '--json'], env=env))
                app = output / result['artifacts'][0]['path']
                executable = next((app / 'Contents/MacOS').iterdir())
                modes = ('available', 'cancel', 'invalid') if version == '1.0.0' else ('current',)
                for mode in modes:
                    server.feed = data.replace(b'1.0.1', b'1.0.2') if mode == 'invalid' else data
                    print(run([executable, mode], cwd='/').strip())
            assert requests and all(path == '/appcast.xml' for path in requests), 'probe must not download an installer'
    finally:
        server.shutdown()
        server.server_close()
        thread.join()

if __name__ == '__main__':
    main()
