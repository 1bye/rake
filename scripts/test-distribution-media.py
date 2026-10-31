#!/usr/bin/env python3
"""Explicit macOS media acceptance using locally installed FFmpeg and GStreamer.

Copies only a test payload into temporary vendor inputs. No network/credentials.
Exercises private helpers, selected real plugins, native dependency closure,
relocation, ad-hoc signing and an empty per-test GStreamer registry.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def run(argv, **options):
    result = subprocess.run([str(arg) for arg in argv], stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True, timeout=900, **options)
    if result.returncode:
        raise RuntimeError(f'{argv[0]} failed: {result.stdout[-8000:]} {result.stderr[-8000:]}')
    return result.stdout


def vendor(source, destination, libraries, search, copied):
    source = source.resolve()
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination in copied:
        if copied[destination] != source:
            raise RuntimeError(f'Native filename collision: {destination}')
        return
    copied[destination] = source
    shutil.copy2(source, destination)
    for line in run(['otool', '-L', source]).splitlines():
        if not line.startswith('\t'):
            continue
        dependency = line.strip().split(' (compatibility version', 1)[0]
        if dependency.startswith(('/usr/lib/', '/System/Library/')):
            continue
        if dependency.startswith('@rpath/'):
            relative = dependency[len('@rpath/'):]
            candidates = [search / relative, source.parent / relative, source.parent / Path(relative).name]
        elif dependency.startswith('@loader_path/'):
            candidates = [source.parent / dependency[len('@loader_path/'):]]
        else:
            candidates = [Path(dependency)]
        resolved = next((path.resolve() for path in candidates if path.is_file()), None)
        if resolved is None:
            raise RuntimeError(f'Cannot prepare dependency {dependency} of {source}')
        if resolved == source:
            continue
        vendor(resolved, libraries / Path(dependency).name, libraries, search, copied)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ffmpeg', type=Path, required=True)
    parser.add_argument('--gstreamer', type=Path, required=True, help='GStreamer framework version root')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='rake-media-') as temporary:
        root = Path(temporary)
        (root / 'src').mkdir()
        target = next(line.split(': ', 1)[1] for line in run(['rustc', '-vV']).splitlines() if line.startswith('host: '))
        copied = {}
        vendor(args.ffmpeg, root / 'vendor/ffmpeg/bin/ffmpeg', root / 'vendor/ffmpeg/runtime/lib', args.ffmpeg.parent.parent, copied)
        gst = args.gstreamer.resolve()
        vendor(gst / 'bin/gst-inspect-1.0', root / 'vendor/gst/bin/gst-inspect', root / 'vendor/gst/runtime/lib', gst, copied)
        vendor(gst / 'libexec/gstreamer-1.0/gst-plugin-scanner', root / 'vendor/gst/runtime/libexec/gstreamer-1.0/gst-plugin-scanner', root / 'vendor/gst/runtime/lib', gst, copied)
        vendor(gst / 'lib/gstreamer-1.0/libgstcoreelements.dylib', root / 'vendor/gst/runtime/lib/gstreamer-1.0/libgstcoreelements.dylib', root / 'vendor/gst/runtime/lib', gst, copied)
        (root / "vendor/ffmpeg/NOTICE").write_text("Acceptance fixture notice")
        (root / "vendor/ffmpeg/presets").mkdir()
        (root / "vendor/ffmpeg/presets/test.txt").write_text("bundled resource")
        (root / 'Cargo.toml').write_text(f'''[workspace]
[package]
name="media-fixture"
version="1.0.0"
edition="2024"
[package.metadata.rake.bundle]
schema_version=1
identifier="dev.rake.media.fixture"
[package.metadata.rake.bundle.dependencies.ffmpeg]
path="vendor/ffmpeg"
executables={{ffmpeg="bin/ffmpeg{{exe}}"}}
licenses=["NOTICE"]
resources={{presets="presets"}}
libraries={{path="runtime"}}
[package.metadata.rake.bundle.dependencies.gst]
path="vendor/gst"
executables={{gst-inspect="bin/gst-inspect{{exe}}"}}
libraries={{path="runtime",recipe="gstreamer",plugins=["coreelements"]}}
''')
        (root / 'src/main.rs').write_text('fn main(){}')
        run(['cargo', 'generate-lockfile', '--offline', '--manifest-path', root / 'Cargo.toml'])
        env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / 'target/distribution-tests'))
        result = json.loads(run([ROOT / 'target/debug/rake', 'bundle', '--manifest-path', root / 'Cargo.toml', '--format', 'app', '--debug', '--output', root / 'output', '--json'], env=env))
        app = root / 'output' / result['artifacts'][0]['path']
        relocated = root / 'relocated/Media.app'
        shutil.copytree(app, relocated, symlinks=True)
        run(['codesign', '--verify', '--deep', '--strict', relocated])
        resources = relocated / 'Contents/Resources'
        assert (resources / 'rake/licenses/ffmpeg/NOTICE').read_text() == 'Acceptance fixture notice'
        assert (resources / 'presets/test.txt').read_text() == 'bundled resource'
        ffmpeg = resources / 'rake/helpers/ffmpeg'
        run([ffmpeg, '-hide_banner', '-f', 'lavfi', '-i', 'testsrc=size=32x32:rate=1', '-frames:v', '1', '-f', 'null', '-'], cwd='/', env={'PATH': '/usr/bin:/bin', 'HOME': str(root)})
        runtime = resources / 'rake/runtimes/gst-runtime'
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(root), 'GST_PLUGIN_SYSTEM_PATH_1_0': '', 'GST_PLUGIN_PATH_1_0': str(runtime / 'lib/gstreamer-1.0'), 'GST_PLUGIN_SCANNER_1_0': str(runtime / 'libexec/gstreamer-1.0/gst-plugin-scanner'), 'GST_REGISTRY_1_0': str(root / 'registry.bin')}
        inspected = run([resources / 'rake/helpers/gst-inspect', 'fakesink'], cwd='/', env=env)
        if str(runtime) not in inspected:
            raise RuntimeError('GStreamer did not load the packaged plugin')
        args.output.write_text(json.dumps({'target': target, 'native_files': len(copied), 'verified': ['relocated FFmpeg processes a frame', 'GStreamer loads selected packaged coreelements plugin', 'private scanner with empty registry', 'strict ad-hoc signature verification', 'no developer working directory or PATH']}, indent=2) + '\n')
        print(args.output.read_text())


if __name__ == '__main__':
    main()
