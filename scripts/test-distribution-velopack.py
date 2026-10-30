#!/usr/bin/env python3
"""Install/upgrade/uninstall an isolated Velopack test app on a disposable Windows CI runner."""
import argparse
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import time


def run(*args, **kwargs):
    return subprocess.check_output([str(arg) for arg in args], text=True, timeout=kwargs.pop("timeout",120), **kwargs)


def wait_for(path, present=True):
    deadline = time.monotonic() + 60
    while path.exists() != present:
        if time.monotonic() >= deadline:
            raise AssertionError(f"installation state did not converge: {path}")
        time.sleep(0.2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=pathlib.Path, required=True)
    args = parser.parse_args()
    assert os.name == "nt", "requires a disposable Windows runner"
    cli = args.cli.resolve()
    repository = pathlib.Path(__file__).resolve().parents[1]
    identifier = f"dev.rake.velosmoke{os.getpid()}"
    install = pathlib.Path(os.environ["LOCALAPPDATA"]) / identifier
    assert not install.exists(), "refuse to replace an existing installation"
    executable = install / "current/velosmoke.exe"
    user_data = pathlib.Path(os.environ["APPDATA"]) / identifier
    assert not user_data.exists(), "refuse to reuse existing application data"
    user_data.mkdir()
    with tempfile.TemporaryDirectory(prefix="rake-velopack-acceptance-") as directory:
        root = pathlib.Path(directory)
        source = root / "source"
        (source / "src").mkdir(parents=True)
        (source / "message.txt").write_text("resource survived installation")
        (source / "build.rs").write_text("fn main(){rake_build::build().unwrap();}")
        (source / "src/main.rs").write_text('fn main(){rake::early_startup!();let r=rake::resources!().unwrap();println!("{}:{}",env!("CARGO_PKG_VERSION"),std::fs::read_to_string(r.path("message.txt").unwrap()).unwrap());}')
        try:
            for version in ("1.0.0", "1.0.1"):
                manifest = f'''[workspace]
[package]
name="velosmoke"
version="{version}"
edition="2024"
[dependencies]
rake={{path={json.dumps(str(repository / "crates/rake"))},default-features=false}}
[build-dependencies]
rake-build={{path={json.dumps(str(repository / "crates/rake-build"))}}}
[package.metadata.rake.bundle]
schema_version=1
identifier="{identifier}"
resources=[{{from="message.txt",to="message.txt"}}]
'''
                (source / "Cargo.toml").write_text(manifest)
                run("cargo", "generate-lockfile", "--offline", "--manifest-path", source / "Cargo.toml")
                output = root / version
                result = json.loads(run(cli, "bundle", "--native", "--debug", "--format", "velopack", "--manifest-path", source / "Cargo.toml", "--output", output, "--certificate-thumbprint", os.environ["RAKE_TEST_CERT_THUMBPRINT"], "--json", timeout=420))
                files = [output / record["path"] for record in result["artifacts"]]
                setup = next(path for path in files if path.name.lower().endswith("setup.exe"))
                assert any(path.suffix == ".nupkg" for path in files)
                assert any(path.name == "releases.stable.json" for path in files)
                run(setup, "--silent")
                wait_for(executable)
                assert run(executable, cwd=root).strip() == f"{version}:resource survived installation"
                marker = user_data / "rake-test-user-state"
                if version == "1.0.0":
                    marker.write_text("preserve me")
                else:
                    assert marker.read_text() == "preserve me"
        finally:
            updater = install / "Update.exe"
            if updater.exists():
                run(updater, "uninstall", "--silent")
                wait_for(executable, present=False)
            marker = user_data / "rake-test-user-state"
            if marker.exists():
                assert marker.read_text() == "preserve me"
            shutil.rmtree(user_data)
        print("Velopack native packaging, startup hooks, install, upgrade and uninstall passed")


if __name__ == "__main__":
    main()
