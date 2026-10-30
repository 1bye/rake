#!/usr/bin/env python3
"""Conventional installer lifecycle and installed GPUI frame on disposable Windows."""
import argparse
import json
import os
import pathlib
import subprocess
import tempfile
import time


def run(*args, **kwargs):
    return subprocess.check_output([str(value) for value in args], text=True, timeout=kwargs.pop('timeout', 420), **kwargs)


def msi(*args):
    result = subprocess.run(['msiexec.exe', *map(str, args), '/qn', '/norestart'], timeout=90)
    assert result.returncode in (0, 3010), f'MSI failed: {result.returncode}'


def absent(path):
    deadline = time.monotonic() + 60
    while path.exists():
        assert time.monotonic() < deadline, f'uninstall retained {path}'
        time.sleep(0.1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=pathlib.Path, required=True)
    args = parser.parse_args()
    assert os.name == 'nt' and os.environ.get('GITHUB_ACTIONS') == 'true', 'requires disposable Windows Actions runner'
    cli = args.cli.resolve()
    repository = pathlib.Path(__file__).resolve().parents[1]
    os.environ['CARGO_TARGET_DIR'] = str(repository / 'target')
    with tempfile.TemporaryDirectory(prefix='rake-installers-') as temporary:
        root = pathlib.Path(temporary)
        source = root / 'source'
        (source / 'src').mkdir(parents=True)
        (source / 'message.txt').write_text('installer resource')
        (source / 'build.rs').write_text('fn main(){rake_build::build().unwrap();}')
        (source / 'src/main.rs').write_text('fn main(){let r=rake_resources::resources!().unwrap();println!("{}:{}",env!("CARGO_PKG_VERSION"),std::fs::read_to_string(r.path("message.txt").unwrap()).unwrap());}')
        for format in ('nsis', 'msi'):
            installed = root / f'installed-{format}'
            executable = installed / 'installer-smoke.exe'
            user = root / f'user-{format}'
            user.mkdir()
            (user / 'document').write_text('preserved user content')
            last_package = None
            try:
                for version in ('1.0.0', '1.0.1'):
                    (source / 'Cargo.toml').write_text(f'''[workspace]
[package]
name="installer-smoke"
version="{version}"
edition="2024"
[dependencies]
rake-resources={{path={json.dumps(str(repository / 'crates/rake-resources'))}}}
[build-dependencies]
rake-build={{path={json.dumps(str(repository / 'crates/rake-build'))}}}
[package.metadata.rake.bundle]
schema_version=1
identifier="dev.rake.installer-{format}-{os.getpid()}"
resources=[{{from="message.txt",to="message.txt"}}]
''')
                    run('cargo', 'generate-lockfile', '--offline', '--manifest-path', source / 'Cargo.toml')
                    output = root / f'{format}-{version}'
                    result = json.loads(run(cli, 'bundle', '--native', '--debug', '--format', format, '--manifest-path', source / 'Cargo.toml', '--output', output, '--certificate-thumbprint', os.environ['RAKE_TEST_CERT_THUMBPRINT'], '--json'))
                    last_package = next(output / item['path'] for item in result['artifacts'])
                    if format == 'msi':
                        msi('/i', last_package, f'INSTALLDIR={installed}')
                    else:
                        run(last_package, '/S', f'/D={installed}')
                    assert run(executable, cwd=root).strip() == f'{version}:installer resource'
                    assert (installed / 'vcruntime140.dll').is_file(), 'CRT must deploy beside the application'
                    assert (user / 'document').read_text() == 'preserved user content'
            finally:
                if executable.exists():
                    if format == 'msi':
                        msi('/x', last_package)
                    else:
                        run(installed / 'uninstall.exe', '/S')
                    absent(executable)
                assert (user / 'document').read_text() == 'preserved user content'
            print(f'{format} signed install, unrelated-CWD resources, N+1 upgrade and uninstall passed', flush=True)
        output = root / 'gpui-output'
        result = json.loads(run(cli, 'bundle', '--native', '--debug', '--package', 'rake-distribution-example', '--format', 'nsis', '--features', 'native-acceptance', '--output', output, '--json', timeout=900))
        setup = next(output / item['path'] for item in result['artifacts'])
        installed = root / 'installed-gpui'
        run(setup, '/S', f'/D={installed}')
        try:
            with (root / 'gpui.log').open('w') as log:
                process = subprocess.Popen([str(installed / 'rake-distribution-example.exe')], cwd=root, stdout=log, stderr=subprocess.STDOUT)
                try:
                    status = process.wait(timeout=60)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                    raise AssertionError('installed GPUI exceeded 60 seconds:\n' + (root / 'gpui.log').read_text())
            log = (root / 'gpui.log').read_text()
            print(log, flush=True)
            assert status == 0 and 'RAKE_NATIVE_FRAME:' in log and 'This resource is included' in log
            print('Windows installed NSIS GPUI native frame and packaged resource passed')
        finally:
            run(installed / 'uninstall.exe', '/S')
            absent(installed / 'rake-distribution-example.exe')


if __name__ == '__main__':
    main()
