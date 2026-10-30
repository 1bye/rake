#!/usr/bin/env python3
"""Real native packager adoption probes, isolated from the Rake runtime.

Requires Rust 1.95, registry access and native installer tools. Unsigned fixtures
exercise packaging; they do not establish signing, installation or updater gates.
"""
import argparse
import hashlib
import json
import plistlib
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / 'tests/distribution-adoption/Cargo.toml'


def run(argv, **kwargs):
    result = subprocess.run([str(item) for item in argv], text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            timeout=1200, **kwargs)
    if result.returncode:
        raise RuntimeError(f'{argv[0]} exited {result.returncode}\n{result.stdout}\n{result.stderr}')
    return result.stdout


def icon(directory):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    png = (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 32, 32, 8, 6, 0, 0, 0))
           + chunk(b'IDAT', zlib.compress((b'\0' + bytes([42, 90, 70, 255]) * 32) * 32))
           + chunk(b'IEND', b''))
    (directory / 'icon.png').write_bytes(png)
    (directory / 'icon.ico').write_bytes(struct.pack('<HHH', 0, 1, 1)
        + struct.pack('<BBBBHHII', 32, 32, 0, 0, 1, 32, len(png), 22) + png)


def probe(engine, build_dir):
    target = next(line.split(': ', 1)[1] for line in run(['rustc', '-vV']).splitlines() if line.startswith('host: '))
    run(['cargo', 'build', '--locked', '--manifest-path', MANIFEST, '--features', engine,
         '--target-dir', build_dir], cwd=ROOT)
    adapter = build_dir / 'debug' / ('rake-distribution-adoption.exe' if sys.platform == 'win32' else 'rake-distribution-adoption')
    formats = {'darwin': ['app'], 'win32': ['nsis'], 'linux': ['deb']}[sys.platform]
    results = []
    for package_format in formats:
        with tempfile.TemporaryDirectory(prefix='rake-adoption-') as temporary:
            root = Path(temporary)
            (root / 'bin').mkdir()
            (root / 'assets').mkdir()
            (root / 'assets/message.txt').write_text('packaged resource')
            icon(root)
            (root / 'extra.plist').write_bytes(plistlib.dumps({'RakeAdoptionProbe': 'custom metadata'}))
            if sys.platform == 'darwin':
                framework = root / 'FixtureRuntime.framework'
                version = framework / 'Versions/A'
                (version / 'Resources').mkdir(parents=True)
                (version / 'Resources/Info.plist').write_bytes(plistlib.dumps({
                    'CFBundleIdentifier': 'dev.rake.adoption.runtime', 'CFBundleExecutable': 'FixtureRuntime',
                    'CFBundlePackageType': 'FMWK', 'CFBundleVersion': '1.0'}))
                source = root / 'runtime.rs'
                source.write_text('#[unsafe(no_mangle)] pub extern "C" fn adoption_value()->u32{42}')
                run(['rustc', '--edition', '2024', '--crate-type', 'cdylib', source, '-o', version / 'FixtureRuntime'])
                run(['install_name_tool', '-id', '@rpath/FixtureRuntime.framework/Versions/A/FixtureRuntime', version / 'FixtureRuntime'])
                (framework / 'Versions/Current').symlink_to('A')
                (framework / 'FixtureRuntime').symlink_to('Versions/Current/FixtureRuntime')
                (framework / 'Resources').symlink_to('Versions/Current/Resources')
            for name, source in [('fixture', 'fn main(){let p=std::env::args_os().nth(1).expect("resource path"); print!("{}",std::fs::read_to_string(p).expect("packaged resource"));}'),
                                 ('fixture-helper', 'fn main(){print!("packaged helper");}')]:
                if sys.platform == 'darwin' and name == 'fixture':
                    source = '#[link(name="FixtureRuntime",kind="framework")] unsafe extern "C"{fn adoption_value()->u32;}' + source.replace('fn main(){', 'fn main(){assert_eq!(unsafe{adoption_value()},42);')
                path = root / f'{name}.rs' 
                path.write_text(source)
                flags = ['-L', f'framework={root}', '-C', 'link-arg=-Wl,-rpath,@executable_path/../Frameworks'] if sys.platform == 'darwin' and name == 'fixture' else []
                run(['rustc', '--edition', '2024', path, *flags, '-o', root / 'bin' / (name + ('.exe' if sys.platform == 'win32' else ''))])
            before = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in (root / 'bin').iterdir()}
            # Windows engines may look for an ICO independently of the PNG list.
            (root / 'bin/icons').mkdir()
            shutil.copyfile(root / 'icon.ico', root / 'bin/icons/icon.ico')
            output = run([adapter, root, package_format, target], cwd=root)
            paths = [Path(p) for p in json.loads(output.splitlines()[-1])]
            assert paths, 'no packages returned'
            assert all(p.exists() for p in paths), 'missing declared artifact'
            for name, digest in before.items():
                assert hashlib.sha256((root / 'bin' / name).read_bytes()).hexdigest() == digest, 'input binary modified'
            verified = ['artifact exists', 'Cargo input unchanged']
            if sys.platform == 'darwin':
                app = next(p for p in paths if p.suffix == '.app')
                relocated = root / 'relocated/App.app'
                shutil.copytree(app, relocated, symlinks=True)
                assert (relocated / 'Contents/Frameworks/FixtureRuntime.framework/Versions/Current').is_symlink()
                info = plistlib.loads((relocated / 'Contents/Info.plist').read_bytes())
                assert info['RakeAdoptionProbe'] == 'custom metadata'
                framework = relocated / 'Contents/Frameworks/FixtureRuntime.framework'
                for signable in [framework / 'Versions/A/FixtureRuntime', framework,
                                 relocated / 'Contents/MacOS/fixture-helper', relocated]:
                    run(['codesign', '--force', '--sign', '-', signable])
                run(['codesign', '--verify', '--deep', '--strict', relocated])
                resource = next(relocated.rglob('message.txt'))
                binary = relocated / 'Contents/MacOS/fixture'
                helper = relocated / 'Contents/MacOS/fixture-helper'
                assert run([binary, resource], cwd='/') == 'packaged resource'
                assert run([helper], cwd='/') == 'packaged helper'
                verified += ['relocated binary runs', 'resource included', 'helper runs', 'framework relocation', 'framework symlinks', 'custom plist', 'external ad-hoc signing verification']
            elif sys.platform == 'linux':
                extracted = root / 'extracted'
                run(['dpkg-deb', '--extract', next(p for p in paths if p.suffix == '.deb'), extracted])
                resource = next(extracted.rglob('message.txt'))
                binary = next(p for p in extracted.rglob('fixture') if p.is_file())
                helper = next(p for p in extracted.rglob('fixture-helper') if p.is_file())
                assert run([binary, resource], cwd='/') == 'packaged resource'
                assert run([helper], cwd='/') == 'packaged helper'
                verified += ['extracted binary runs', 'resource included', 'helper runs']
            results.append({'engine': engine, 'target': target, 'format': package_format, 'verified': verified,
                            'pending': ['native installation/upgrade', 'Developer ID/notarization', 'non-macOS library relocation', 'cancellation', 'update handoff']})
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', choices=['tauri-bundler', 'cargo-packager', 'both'], default='both')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    engines = ['tauri-bundler', 'cargo-packager'] if args.engine == 'both' else [args.engine]
    results = []
    for engine in engines:
        results.extend(probe(engine, ROOT / 'tests/distribution-adoption/target'))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    main()
