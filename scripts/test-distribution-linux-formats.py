#!/usr/bin/env python3
"""Remaining Arch and real Flatpak GPUI gates on a disposable Linux runner."""
import argparse
import hashlib
import signal
import json
import os
import pathlib
import subprocess
import tempfile


def run(*args, **kwargs):
    return subprocess.check_output([str(arg) for arg in args], text=True, timeout=1200, **kwargs)


def arch(cli, repository, root):
    source = root / 'arch-source'
    (source / 'src').mkdir(parents=True)
    (source / 'message.txt').write_text('Arch installed resource')
    (source / 'src/main.rs').write_text('fn main(){let r=rake_resources::resources!().unwrap();println!("{}:{}",env!("CARGO_PKG_VERSION"),std::fs::read_to_string(r.path("message.txt").unwrap()).unwrap());}')
    (source / 'build.rs').write_text('fn main(){rake_build::build().unwrap();}')
    (source / 'Cargo.toml').write_text(f'''[workspace]
[package]
name="rake-arch-smoke"
version="1.0.0"
edition="2024"
[dependencies]
rake-resources={{path={json.dumps(str(repository / 'crates/rake-resources'))}}}
[build-dependencies]
rake-build={{path={json.dumps(str(repository / 'crates/rake-build'))}}}
[package.metadata.rake.bundle]
schema_version=1
identifier="dev.rake.arch-smoke"
resources=[{{from="message.txt",to="message.txt"}}]
[package.metadata.rake.bundle.linux]
formats=["arch"]
[package.metadata.rake.bundle.linux.dependencies]
arch=["glibc"]
''')
    runner = root / 'arch.py'
    runner.write_text(r'''
import json,os,pathlib,subprocess,sys
cli,source,root=map(pathlib.Path,sys.argv[1:]);os.environ['CARGO_TARGET_DIR']=str(root/'arch-target')
def run(*args,**kwargs):return subprocess.check_output([str(arg) for arg in args],text=True,**kwargs)
outputs=[];packages=[]
for version in ('1.0.0','1.0.1'):
    manifest=source/'Cargo.toml'
    manifest.write_text(manifest.read_text().replace('version="1.0.0"',f'version="{version}"'))
    run('cargo','generate-lockfile','--offline','--manifest-path',manifest)
    output=root/version
    result=json.loads(run(cli,'bundle','--native','--debug','--manifest-path',manifest,'--output',output,'--json'))
    package=next(output/record['path'] for record in result['artifacts'] if '.pkg.tar.' in record['path'])
    run('sudo','pacman','--noconfirm','-U',package)
    assert run('/usr/bin/rake-arch-smoke',cwd='/').strip()==f'{version}:Arch installed resource'
    outputs.append(output);packages.append(package)
run('sudo','pacman','--noconfirm','-R','rake-arch-smoke')
assert not pathlib.Path('/usr/bin/rake-arch-smoke').exists()
home=root/'arch-gpg';home.mkdir(mode=0o700);os.environ['GNUPGHOME']=str(home)
run('gpg','--batch','--pinentry-mode','loopback','--passphrase','','--quick-generate-key','Rake Arch acceptance','rsa2048','sign','1d')
fingerprint=next(line.split(':')[9] for line in run('gpg','--batch','--with-colons','--list-keys').splitlines() if line.startswith('fpr:'))
with (source/'Cargo.toml').open('a') as file:
    file.write(f'\n[package.metadata.rake.bundle.release]\ngpg_key="{fingerprint}"\n[package.metadata.rake.bundle.release.publisher]\nkind="directory"\npath={json.dumps(str(root/"published"))}\npublic_url="https://arch.example.test"\n[package.metadata.rake.bundle.release.required]\nx86_64-unknown-linux-gnu=["arch"]\n[package.metadata.rake.bundle.release.repositories]\nx86_64-unknown-linux-gnu=["arch"]\n')
run(cli,'release','--manifest-path',source/'Cargo.toml','--from',outputs[0],'--from',outputs[1],'--json')
repo=root/'published/stable/x86_64-unknown-linux-gnu/arch'
assert (repo/'rake-arch.db').is_file()
run('sudo','pacman-key','--init')
run('sudo','pacman-key','--add',repo/'public-key.asc')
run('sudo','pacman-key','--lsign-key',fingerprint)
config=root/'pacman.conf'
config.write_text(f'[options]\nSigLevel = Required DatabaseRequired\nLocalFileSigLevel = Required\n[rake-arch]\nServer = file://{repo}\n')
run('sudo','pacman','--noconfirm','-U',packages[0])
run('sudo','pacman','--config',config,'--noconfirm','-Sy','rake-arch-smoke')
assert run('/usr/bin/rake-arch-smoke',cwd='/').strip()=='1.0.1:Arch installed resource'
run('sudo','pacman','--noconfirm','-R','rake-arch-smoke')
print('Arch native makepkg, install, upgrade, uninstall and signed repository discovery passed')
''')
    cargo = pathlib.Path(os.environ.get('CARGO_HOME', pathlib.Path.home() / '.cargo'))
    toolchain = run('rustc', '--print', 'sysroot').strip()
    command = ['docker', 'run', '--rm', '-v', f'{repository}:{repository}:ro', '-v', f'{root}:{root}', '-v', f'{toolchain}:/opt/rust:ro', '-v', f'{cargo / "registry"}:/cargo/registry', '-v', f'{cargo / "git"}:/cargo/git', '-e', f'TEST_UID={os.getuid()}', '-e', f'TEST_CLI={cli}', '-e', f'TEST_SOURCE={source}', '-e', f'TEST_ROOT={root}', '-e', f'TEST_RUNNER={runner}', 'archlinux:base-devel', 'bash', '-ec', r'''
pacman -Syu --noconfirm python patchelf sudo git
useradd -m -u "$TEST_UID" builder
printf 'builder ALL=(ALL) NOPASSWD: ALL\n' > /etc/sudoers.d/rake-acceptance
install -d -m 0777 /cargo
sudo -u builder env PATH=/opt/rust/bin:/usr/bin CARGO_HOME=/cargo CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 python "$TEST_RUNNER" "$TEST_CLI" "$TEST_SOURCE" "$TEST_ROOT"
''']
    print(run(*command))


def flatpak_ui(cli, repository, root, runtime, sdk, reuse=False):
    manifest = repository / 'examples/distribution/Cargo.toml'
    original = manifest.read_text()
    output = repository / 'target/distribution-native-flatpak'
    if reuse:
        result = json.loads((output / 'distribution.json').read_text())
    else:
        try:
            manifest.write_text(original + f'\n[package.metadata.rake.bundle.linux.flatpak]\nruntime_commit="{runtime}"\nsdk_commit="{sdk}"\n')
            result = json.loads(run(cli, 'bundle', '--native', '--package', 'rake-distribution-example', '--format', 'flatpak', '--features', 'native-acceptance', '--output', output, '--json'))
        finally:
            manifest.write_text(original)
    package = next(output / record['path'] for record in result['artifacts'] if record['path'].endswith('.flatpak'))
    record = next(record for record in result['artifacts'] if record['path'].endswith('.flatpak'))
    assert hashlib.file_digest(package.open('rb'), 'sha256').hexdigest() == record['sha256'], 'reused Flatpak bytes must match their build record'
    identifier = 'dev.rake.desktop-example'
    run('flatpak', 'install', '--user', '-y', '--noninteractive', package)
    try:
        launcher = root / 'flatpak-launch.sh'
        launcher.write_text(r"""set -eu
openbox >"$1/window-manager.log" 2>&1 &
manager=$!
trap 'kill "$manager" 2>/dev/null || true' EXIT
flatpak run --user --no-documents-portal --no-a11y-bus --env=LIBGL_ALWAYS_SOFTWARE=1 dev.rake.desktop-example
""")
        assert_frame(['xvfb-run', '-a', 'dbus-run-session', '--', 'sh', str(launcher), str(root)], output / 'native-frame.log')
        runtime_dir = root / 'wayland-runtime'
        runtime_dir.mkdir(mode=0o700)
        wayland = root / 'flatpak-wayland.sh'
        wayland.write_text(r"""set -eu
export XDG_RUNTIME_DIR="$1"
export WAYLAND_DISPLAY=rake-wayland
weston --backend=x11-backend.so --use-pixman --socket="$WAYLAND_DISPLAY" --idle-time=0 --log="$2/weston.log" &
manager=$!
trap 'kill "$manager" 2>/dev/null || true' EXIT
for attempt in $(seq 1 100); do
    test ! -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" || break
    kill -0 "$manager" || { cat "$2/weston.log"; exit 1; }
    sleep 0.05
done
test -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
flatpak run --user --no-documents-portal --no-a11y-bus --env=DISPLAY= --env=WAYLAND_DISPLAY="$WAYLAND_DISPLAY" --env=LIBGL_ALWAYS_SOFTWARE=1 dev.rake.desktop-example
""")
        assert_frame(['xvfb-run', '-a', 'dbus-run-session', '--', 'sh', str(wayland), str(runtime_dir), str(output)], output / 'native-wayland-frame.log')
        print('Flatpak pinned SDK source build, X11/Wayland sandbox GPUI frames and installed resource passed', flush=True)
    finally:
        run('flatpak', 'uninstall', '--user', '-y', '--noninteractive', identifier)


def assert_frame(command, log_path):
    with log_path.open('w') as log_file:
        process = subprocess.Popen(command, stdout=log_file, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            status = process.wait(timeout=60)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            raise AssertionError('Flatpak UI exceeded 60 seconds:\n' + log_path.read_text())
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
    log = log_path.read_text()
    print(log, flush=True)
    assert status == 0, 'Flatpak process failed'
    assert 'RAKE_NATIVE_FRAME:' in log, log
    assert 'This resource is included' in log, log


def flatpak_repository(cli, repository, root, runtime, sdk):
    manifest = repository / 'examples/distribution/Cargo.toml'
    lock = repository / 'Cargo.lock'
    fixture = manifest.parent / 'native-repository.rs'
    assert not fixture.exists(), 'refuse to replace an existing source file'
    original, original_lock = manifest.read_bytes(), lock.read_bytes()
    home = root / 'flatpak-gpg'
    home.mkdir(mode=0o700)
    previous_home = os.environ.get('GNUPGHOME')
    os.environ['GNUPGHOME'] = str(home)
    identifier = 'dev.rake.RepositorySmoke'
    remote = f'rake-repository-{os.getpid()}'
    installed = False
    try:
        run('gpg', '--batch', '--pinentry-mode', 'loopback', '--passphrase', '', '--quick-generate-key', 'Rake Flatpak acceptance', 'rsa2048', 'sign', '1d')
        fingerprint = next(line.split(':')[9] for line in run('gpg', '--batch', '--with-colons', '--list-keys').splitlines() if line.startswith('fpr:'))
        fixture.write_text(r"""fn main(){
let r=rake_resources::resources!().unwrap();
let data=std::path::PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap());std::fs::create_dir_all(&data).unwrap();
let state=data.join("rake-test-state");if state.exists(){assert_eq!(std::fs::read_to_string(&state).unwrap(),"preserve me");}else{std::fs::write(&state,"preserve me").unwrap();}
println!("{}:{}",env!("CARGO_PKG_VERSION"),std::fs::read_to_string(r.path("welcome.txt").unwrap()).unwrap().trim());
}""")
        for version in ('1.0.0', '1.0.1'):
            manifest.write_text(f"""[package]
name="rake-distribution-example"
version="{version}"
edition.workspace=true
rust-version.workspace=true
license.workspace=true
publish=false
autobins=false
[[bin]]
name="rake-repository-smoke"
path="native-repository.rs"
[dependencies]
rake-resources={{path="../../crates/rake-resources"}}
[build-dependencies]
rake-build={{path="../../crates/rake-build"}}
[package.metadata.rake.bundle]
schema_version=1
identifier="{identifier}"
resources=[{{from="assets/welcome.txt",to="welcome.txt"}}]
[package.metadata.rake.bundle.linux]
formats=["flatpak"]
[package.metadata.rake.bundle.linux.flatpak]
runtime_commit="{runtime}"
sdk_commit="{sdk}"
finish_args=[]
[package.metadata.rake.bundle.release]
gpg_key="{fingerprint}"
[package.metadata.rake.bundle.release.publisher]
kind="directory"
path={json.dumps(str(root / 'flatpak-published'))}
public_url="https://flatpak.example.test"
[package.metadata.rake.bundle.release.required]
x86_64-unknown-linux-gnu=["flatpak"]
[package.metadata.rake.bundle.release.repositories]
x86_64-unknown-linux-gnu=["flatpak"]
""")
            run('cargo', 'generate-lockfile', '--offline', '--manifest-path', manifest)
            output = root / f'flatpak-{version}'
            run(cli, 'bundle', '--native', '--package', 'rake-distribution-example', '--bin', 'rake-repository-smoke', '--manifest-path', manifest, '--output', output, '--json')
            exported = root / f'repositories-{version}'
            run(cli, 'release', '--package', 'rake-distribution-example', '--manifest-path', manifest, '--from', output, '--prepare-repositories', exported, '--json')
            run(cli, 'release', '--package', 'rake-distribution-example', '--manifest-path', manifest, '--from', output, '--repositories', exported, '--json')
            published = root / 'flatpak-published/stable/x86_64-unknown-linux-gnu/flatpak'
            if version == '1.0.0':
                reference = (published / 'rake-flatpak.flatpakref').read_text()
                assert f'Name={identifier}' in reference and 'GPGKey=' in reference and 'Branch=master' in reference
                run('flatpak', 'remote-add', '--user', '--gpg-import', published / 'public-key.asc', remote, (published / 'repo').as_uri())
                run('flatpak', 'install', '--user', '-y', '--noninteractive', remote, f'{identifier}//master')
                installed = True
            else:
                run('flatpak', 'update', '--user', '-y', '--noninteractive', identifier)
            result = run('dbus-run-session', '--', 'flatpak', 'run', '--user', '--no-documents-portal', '--no-a11y-bus', identifier)
            assert result.startswith(f'{version}:This resource is included'), result
        print('Flatpak signed repository/ref enrollment, fresh N install, N+1 discovery/upgrade and preserved app data passed', flush=True)
    finally:
        try:
            if installed:
                run('flatpak', 'uninstall', '--user', '-y', '--noninteractive', '--delete-data', identifier)
                run('flatpak', 'remote-delete', '--user', '--force', remote)
        finally:
            manifest.write_bytes(original)
            lock.write_bytes(original_lock)
            fixture.unlink(missing_ok=True)
            if previous_home is None:
                os.environ.pop('GNUPGHOME', None)
            else:
                os.environ['GNUPGHOME'] = previous_home


def flatpak(cli, repository, root, reuse=False):
    assert not run('flatpak', 'list', '--user', '--app', '--columns=application').strip(), 'use an isolated Flatpak test user'
    run('flatpak', 'remote-add', '--user', '--if-not-exists', 'flathub', 'https://dl.flathub.org/repo/flathub.flatpakrepo')
    run('flatpak', 'install', '--user', '-y', '--noninteractive', 'flathub', 'org.freedesktop.Platform//25.08', 'org.freedesktop.Sdk//25.08')
    runtime = run('flatpak', 'info', '--user', '--show-commit', 'org.freedesktop.Platform/x86_64/25.08').strip()
    sdk = run('flatpak', 'info', '--user', '--show-commit', 'org.freedesktop.Sdk/x86_64/25.08').strip()
    failures = []
    actions = [('native UI', lambda: flatpak_ui(cli, repository, root, runtime, sdk, reuse))]
    actions.append(('signed repository', lambda: flatpak_repository(cli, repository, root, runtime, sdk)))
    for name, action in actions:
        try:
            action()
        except Exception as error:
            failures.append(f'{name}: {error}')
            print(f'Flatpak {name} failed: {error}', flush=True)
    if failures:
        raise AssertionError('\n'.join(failures))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=pathlib.Path, required=True)
    parser.add_argument('--only', choices=['arch', 'flatpak'])
    parser.add_argument('--reuse-flatpak', action='store_true', help='reuse an explicitly downloaded, hash-verified GUI artifact; repository fixtures still build from source')
    args = parser.parse_args()
    assert os.uname().sysname == 'Linux' and os.environ.get('GITHUB_ACTIONS') == 'true', 'requires a disposable Linux Actions runner'
    cli = args.cli.resolve()
    repository = pathlib.Path(__file__).resolve().parents[1]
    os.environ['CARGO_TARGET_DIR'] = str(repository / 'target')
    failures = []
    with tempfile.TemporaryDirectory(prefix='rake-linux-formats-') as directory:
        root = pathlib.Path(directory)
        for name, action in [('arch', arch), ('flatpak', lambda cli, repository, root: flatpak(cli, repository, root, args.reuse_flatpak))]:
            if args.only and args.only != name:
                continue
            try:
                action(cli, repository, root)
            except Exception as error:
                failures.append(f'{name}: {error}')
                print(f'{name} acceptance failed: {error}', flush=True)
    if failures:
        raise SystemExit('\n'.join(failures))


if __name__ == '__main__':
    main()
