#!/usr/bin/env python3
"""Spawn the portable CLI against isolated publication fixtures (no live uploads).

This checks planning, immutable export and replay refusal. Native package-manager
and updater acceptance is separate: this script deliberately uses opaque files.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=pathlib.Path, required=True)
    args = parser.parse_args()
    cli = args.cli.resolve()
    with tempfile.TemporaryDirectory(prefix='rake-release-cli-') as directory:
        root = pathlib.Path(directory)
        (root / 'src').mkdir()
        (root / 'src/main.rs').write_text('fn main(){}')
        (root / 'Cargo.toml').write_text('''[package]
name="release-test"
version="1.0.0"
edition="2024"
[workspace]
[package.metadata.rake.bundle]
schema_version=1
identifier="dev.rake.release-test"
[package.metadata.rake.bundle.release.publisher]
kind="directory"
path="published"
public_url="https://downloads.example.test"
[package.metadata.rake.bundle.release.required]
x86_64-unknown-linux-gnu=["tar"]
''')
        subprocess.run(['cargo', 'generate-lockfile', '--offline', '--manifest-path', str(root / 'Cargo.toml')], check=True, capture_output=True)
        (root / 'input').mkdir()
        payload = b'test immutable publication'
        (root / 'input/app.tar.gz').write_bytes(payload)
        (root / 'input/distribution.json').write_text(json.dumps(dict(schema_version=1, identifier='dev.rake.release-test', version='1.0.0', target='x86_64-unknown-linux-gnu', engine='test', artifacts=[dict(path='app.tar.gz', format='tar', sha256=hashlib.sha256(payload).hexdigest(), package_version='1.0.0')])))
        command = [str(cli), 'release', '--manifest-path', str(root / 'Cargo.toml'), '--from', str(root / 'input'), '--json']
        plan = json.loads(subprocess.check_output(command + ['--plan']))
        assert plan['channel'] == 'stable'
        assert not (root / 'published').exists()
        result = json.loads(subprocess.check_output(command))
        assert result['version'] == '1.0.0'
        assert (root / 'published/stable/release.json').exists()
        replay = subprocess.run(command, capture_output=True, text=True)
        assert replay.returncode != 0 and 'must advance' in replay.stderr
        print('Spawned CLI release plan/publication/replay checks passed')


if __name__ == '__main__':
    main()
