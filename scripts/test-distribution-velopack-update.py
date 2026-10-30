#!/usr/bin/env python3
"""Signed HTTPS publication and SDK-driven N-to-N+1 on a disposable Windows runner."""
import argparse
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import time
import winreg
from distribution_test_feed import TestFeed


def run(*args, **kwargs):
    return subprocess.check_output([str(value) for value in args], text=True, timeout=kwargs.pop('timeout', 420), **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=pathlib.Path, required=True)
    args = parser.parse_args()
    assert os.name == 'nt' and os.environ.get('GITHUB_ACTIONS') == 'true', 'requires disposable Windows Actions runner'
    cli = args.cli.resolve()
    repository = pathlib.Path(__file__).resolve().parents[1]
    os.environ['CARGO_TARGET_DIR'] = str(repository / 'target')
    identifier = f'dev.rake.veloupdate{os.getpid()}'
    install = pathlib.Path(os.environ['LOCALAPPDATA']) / identifier
    user = pathlib.Path(os.environ['APPDATA']) / identifier
    assert not install.exists() and not user.exists(), 'refuse to replace existing installation or user data'
    user.mkdir()
    (user / 'document').write_text('preserve dirty work')
    try:
        with tempfile.TemporaryDirectory(prefix='rake-velopack-update-') as temporary:
            root = pathlib.Path(temporary)
            with TestFeed(root) as feed:
                source = root / 'source'
                (source / 'src').mkdir(parents=True)
                (source / 'ca.pem').write_bytes(feed.ca.read_bytes())
                seed = feed.private / 'rake-update-seed'
                run(cli, 'update-key', 'generate', '--private-key', seed, '--public-key', source / 'updates.pub')
                (source / 'message.txt').write_text('installed update resource')
                (source / 'build.rs').write_text('fn main(){rake_build::build().unwrap();}')
                (source / 'src/main.rs').write_text(r'''
use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
fn main(){
    rake::early_startup!();
    let resources=rake::resources!().unwrap();
    let state=std::path::PathBuf::from(std::env::var_os("RAKE_FIXTURE_STATE").unwrap());
    let args=std::env::args().collect::<Vec<_>>();
    let mode=if args.iter().any(|arg|arg=="--after-update"){"--after-update"}else{args.get(1).map(String::as_str).unwrap_or("--version")};
    if mode=="--version" || mode=="--after-update" {
        assert_eq!(std::fs::read_to_string(resources.path("message.txt").unwrap()).unwrap(),"installed update resource");
        if mode=="--after-update" {std::fs::write(state.join("restarted"),env!("CARGO_PKG_VERSION")).unwrap();}
        println!("{}:installed update resource",env!("CARGO_PKG_VERSION"));return;
    }
    let service=rake::updates!().unwrap().testing_loopback_tls(&std::fs::read(resources.path("ca.pem").unwrap()).unwrap()).unwrap();
    let cancelled=Arc::new(AtomicBool::new(false));
    let update=service.check_velopack(Arc::clone(&cancelled));
    if mode=="--reject-feed" {assert!(update.is_err());return;}
    let update=update.unwrap().expect("N+1 update");
    if mode=="--check" {println!("available:{}",update.version());return;}
    let cancel=Arc::clone(&cancelled);
    let cancel_transfer=mode=="--cancel";
    let staged=service.stage_velopack(update,cancelled,move |_|{if cancel_transfer{cancel.store(true,Ordering::Relaxed);}});
    if cancel_transfer {assert!(staged.is_err());return;}
    if mode=="--reject-cache" {assert!(staged.is_err());return;}
    let handoff=staged.unwrap().restart_handoff(vec!["--after-update".into()]);
    if mode=="--decline" {assert!(handoff.commit(false).is_err());return;}
    handoff.commit(true).unwrap();
}
''')
                outputs = []
                setup = None
                for version in ('1.0.0', '1.0.1'):
                    manifest = f'''[workspace]
[package]
name="veloupdate"
version="{version}"
edition="2024"
[dependencies]
rake={{path={json.dumps(str(repository / 'crates/rake'))},default-features=false,features=["updates-velopack"]}}
rake-updates={{path={json.dumps(str(repository / 'crates/rake-updates'))},features=["velopack","native-tests"]}}
[build-dependencies]
rake-build={{path={json.dumps(str(repository / 'crates/rake-build'))}}}
[package.metadata.rake.bundle]
schema_version=1
identifier="{identifier}"
resources=[{{from="ca.pem",to="ca.pem"}},{{from="message.txt",to="message.txt"}}]
[package.metadata.rake.bundle.desktop]
url_schemes=["rake-veloupdate{os.getpid()}-{version[-1]}"]
[package.metadata.rake.bundle.windows]
formats=["velopack"]
timestamp_url="http://timestamp.digicert.com"
[package.metadata.rake.bundle.updates]
enabled=true
base_url={json.dumps(feed.base_url)}
[package.metadata.rake.bundle.updates.public_keys]
velopack="updates.pub"
[package.metadata.rake.bundle.release.publisher]
kind="directory"
path={json.dumps(str(feed.site))}
public_url={json.dumps(feed.base_url)}
[package.metadata.rake.bundle.release.signing_keys.velopack]
kind="file"
path={json.dumps(str(seed))}
[package.metadata.rake.bundle.release.required]
x86_64-pc-windows-msvc=["velopack"]
'''
                    (source / 'Cargo.toml').write_text(manifest)
                    run('cargo', 'generate-lockfile', '--offline', '--manifest-path', source / 'Cargo.toml')
                    output = root / version
                    result = json.loads(run(cli, 'bundle', '--native', '--debug', '--manifest-path', source / 'Cargo.toml', '--output', output, '--certificate-thumbprint', os.environ['RAKE_TEST_CERT_THUMBPRINT'], '--json'))
                    outputs.append(output)
                    if version == '1.0.0':
                        setup = next(output / item['path'] for item in result['artifacts'] if item['path'].lower().endswith('setup.exe'))
                run(cli, 'release', '--manifest-path', source / 'Cargo.toml', '--from', outputs[0], '--from', outputs[1], '--json')
                run(setup, '--silent')
                executable = install / 'current/veloupdate.exe'
                deadline = time.monotonic() + 60
                while not executable.exists():
                    assert time.monotonic() < deadline, 'installation did not finish'
                    time.sleep(0.1)
                assert (install / 'current/vcruntime140.dll').is_file(), 'MSVC runtime must be private to the installation'
                env = dict(os.environ, RAKE_FIXTURE_STATE=str(user))
                assert run(executable, cwd=root, env=env).strip() == '1.0.0:installed update resource'
                assert run(executable, '--check', env=env).strip() == 'available:1.0.1'
                native_feed = feed.site / 'stable/x86_64-pc-windows-msvc/releases.stable.json'
                signed_bytes = native_feed.read_bytes()
                # Published channel files share content-addressed objects; replace only
                # the feed alias so the restored immutable object remains untouched.
                native_feed.unlink()
                native_feed.write_bytes(signed_bytes + b' ')
                run(executable, '--reject-feed', env=env)
                native_feed.write_bytes(signed_bytes)
                for mode in ('--cancel', '--decline'):
                    run(executable, mode, env=env)
                    assert run(executable, env=env).startswith('1.0.0:')
                    assert not (user / 'restarted').exists()
                cached = next(path for path in (install / 'packages').glob('*.nupkg') if '1.0.1' in path.name)
                cached.write_bytes(b'corrupted cached package')
                run(executable, '--reject-cache', env=env)
                assert not cached.exists(), 'invalid cached package must not block a clean retry'
                run(executable, '--update', env=env)
                deadline = time.monotonic() + 90
                while not (user / 'restarted').exists():
                    assert time.monotonic() < deadline, 'SDK did not apply and relaunch N+1'
                    time.sleep(0.1)
                assert (user / 'restarted').read_text() == '1.0.1'
                assert run(executable, env=env).strip() == '1.0.1:installed update resource'
                scheme = f'rake-veloupdate{os.getpid()}'
                with winreg.OpenKey(winreg.HKEY_CURRENT_USER, f'Software\\Classes\\{scheme}-1\\shell\\open\\command') as key:
                    assert winreg.QueryValueEx(key, '')[0].casefold() == f'"{executable}" "%1"'.casefold()
                try:
                    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, f'Software\\Classes\\{scheme}-0'):
                        raise AssertionError('SDK update must remove the old protocol registration')
                except FileNotFoundError:
                    pass
                assert (user / 'document').read_text() == 'preserve dirty work'
                print('Velopack built-in signing, HTTPS tamper/cancel/decline/cache rejection, SDK N+1 relaunch and registration migration passed')
    finally:
        updater = install / 'Update.exe'
        if updater.exists():
            run(updater, 'uninstall', '--silent')
        assert (user / 'document').read_text() == 'preserve dirty work'
        shutil.rmtree(user)


if __name__ == '__main__':
    main()
