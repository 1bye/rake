#!/usr/bin/env python3
"""Real full-image update against an isolated, signed HTTPS publication on Linux."""
import argparse
import fcntl
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import time
from distribution_test_feed import TestFeed


def run(*args, **kwargs):
    return subprocess.check_output([str(arg) for arg in args], text=True, timeout=600, **kwargs)


def digest(path):
    return hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=pathlib.Path, required=True)
    args = parser.parse_args()
    assert os.uname().sysname == 'Linux', 'requires a native Linux runner'
    cli = args.cli.resolve()
    repository = pathlib.Path(__file__).resolve().parents[1]
    os.environ['CARGO_TARGET_DIR'] = str(repository / 'target')
    os.environ['APPIMAGE_EXTRACT_AND_RUN'] = '1'
    with tempfile.TemporaryDirectory(prefix='rake-appimage-acceptance-') as temporary:
        root = pathlib.Path(temporary)
        with TestFeed(root) as feed:
            source = root / 'source'
            (source / 'src').mkdir(parents=True)
            (source / 'ca.pem').write_bytes(feed.ca.read_bytes())
            (source / 'updates.pub').write_text(feed.public_key.hex())
            (source / 'message.txt').write_text('installed resource')
            (source / 'build.rs').write_text('fn main(){rake_build::build().unwrap();}')
            (source / 'src/main.rs').write_text(r'''
use std::sync::atomic::{AtomicBool,Ordering};
fn main(){
    rake::early_startup!();
    let resources=rake::resources!().unwrap();
    let state=std::path::PathBuf::from(std::env::var_os("RAKE_FIXTURE_STATE").unwrap());
    let mode=std::env::args().nth(1).unwrap_or_else(||"--version".into());
    if mode=="--version" || mode=="--after-update" {
        assert_eq!(std::fs::read_to_string(resources.path("message.txt").unwrap()).unwrap(),"installed resource");
        if mode=="--after-update" {std::fs::write(state.join("restarted"),env!("CARGO_PKG_VERSION")).unwrap();}
        println!("{}:installed resource",env!("CARGO_PKG_VERSION"));return;
    }
    let service=rake::updates!().unwrap().testing_loopback_tls(&std::fs::read(resources.path("ca.pem").unwrap()).unwrap()).unwrap();
    let cancelled=AtomicBool::new(false);
    let update=service.check_appimage(&cancelled).unwrap().expect("N+1 update");
    if mode=="--check" {println!("available:{}",update.release().version);return;}
    let staged=service.stage_appimage(update,&cancelled,|_|{if mode=="--cancel"{cancelled.store(true,Ordering::Relaxed);}});
    if mode=="--cancel" {assert!(matches!(staged,Err(rake::updates::Error::Cancelled)));return;}
    let handoff=staged.unwrap().restart_handoff(vec!["--after-update".into()]);
    if mode=="--decline" {assert!(handoff.commit(false).is_err());return;}
    handoff.commit(true).unwrap();
}
''')
            outputs = []
            images = []
            for version in ('1.0.0', '1.0.1'):
                manifest = f'''[workspace]
[package]
name="rake-image-smoke"
version="{version}"
edition="2024"
[dependencies]
rake={{path={json.dumps(str(repository / 'crates/rake'))},default-features=false}}
rake-updates={{path={json.dumps(str(repository / 'crates/rake-updates'))},features=["appimage","native-tests"]}}
[build-dependencies]
rake-build={{path={json.dumps(str(repository / 'crates/rake-build'))}}}
[package.metadata.rake.bundle]
schema_version=1
identifier="dev.rake.image-smoke"
resources=[{{from="ca.pem",to="ca.pem"}},{{from="message.txt",to="message.txt"}}]
[package.metadata.rake.bundle.linux]
formats=["appimage"]
[package.metadata.rake.bundle.updates]
enabled=true
base_url={json.dumps(feed.base_url)}
[package.metadata.rake.bundle.updates.public_keys]
appimage="updates.pub"
[package.metadata.rake.bundle.release.publisher]
kind="directory"
path={json.dumps(str(feed.site))}
public_url={json.dumps(feed.base_url)}
[package.metadata.rake.bundle.release.signers]
appimage={json.dumps(feed.signer)}
[package.metadata.rake.bundle.release.required]
x86_64-unknown-linux-gnu=["appimage"]
'''
                (source / 'Cargo.toml').write_text(manifest)
                run('cargo', 'generate-lockfile', '--offline', '--manifest-path', source / 'Cargo.toml')
                output = root / version
                result = json.loads(run(cli, 'bundle', '--native', '--debug', '--manifest-path', source / 'Cargo.toml', '--output', output, '--json'))
                image = next(output / record['path'] for record in result['artifacts'] if record['format'] == 'appimage')
                assert image.read_bytes()[8:11] == b'AI\x02'
                outputs.append(output)
                images.append(image)
            run(cli, 'release', '--manifest-path', source / 'Cargo.toml', '--from', outputs[0], '--from', outputs[1], '--json')
            installed = root / 'installed/App.AppImage'
            installed.parent.mkdir()
            shutil.copy2(images[0], installed)
            original = digest(installed)
            state = root / 'state'
            state.mkdir()
            (state / 'document').write_text('preserve unsaved work')
            env = dict(os.environ, RAKE_FIXTURE_STATE=str(state))
            assert run(installed, '--version', cwd='/', env=env).strip() == '1.0.0:installed resource'
            assert run(installed, '--check', cwd='/', env=env).strip() == 'available:1.0.1'
            for mode in ('--decline', '--cancel'):
                run(installed, mode, cwd='/', env=env)
                assert digest(installed) == original
                assert not list(installed.parent.glob('*.rake-image-*'))
            installed.chmod(0o555)
            rejected = subprocess.run([str(installed), '--update'], env=env, capture_output=True, text=True, timeout=60)
            assert rejected.returncode != 0 and digest(installed) == original
            installed.chmod(0o755)
            with (installed.parent / '.App.AppImage.rake-update.lock').open('w') as lock:
                fcntl.flock(lock, fcntl.LOCK_EX)
                rejected = subprocess.run([str(installed), '--update'], env=env, capture_output=True, text=True, timeout=60)
                assert rejected.returncode != 0 and digest(installed) == original
            run(installed, '--update', cwd='/', env=env)
            deadline = time.monotonic() + 30
            while not (state / 'restarted').exists():
                assert time.monotonic() < deadline, 'updated image did not relaunch'
                time.sleep(0.1)
            assert (state / 'restarted').read_text() == '1.0.1'
            assert digest(installed) == digest(images[1])
            assert (state / 'document').read_text() == 'preserve unsaved work'
            print('Real AppImage signed HTTPS publication, decline, cancel, locks, readonly rejection and N+1 relaunch passed')


if __name__ == '__main__':
    main()
