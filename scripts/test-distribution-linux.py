#!/usr/bin/env python3
"""Native Linux install/upgrade smoke test on disposable CI runners only.

Requires a built rake CLI, cached Cargo dependencies, sudo/dpkg, Docker and RPM.
It installs only the uniquely named rake-distribution-smoke package and removes it
in finally. Fedora runs inside its own disposable container.
"""
import argparse
import json
import pathlib
import shutil
import subprocess
import tempfile


def run(*args, **kwargs):
    return subprocess.check_output([str(arg) for arg in args], text=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=pathlib.Path, required=True)
    parser.add_argument("--install", action="store_true", required=True)
    args = parser.parse_args()
    cli = args.cli.resolve()
    repository = pathlib.Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="rake-linux-acceptance-") as temporary:
        root = pathlib.Path(temporary)
        project = root / "source"
        (project / "src").mkdir(parents=True)
        (project / "message.txt").write_text("installed resource")
        (project / "build.rs").write_text("fn main(){rake_build::build().unwrap();}")
        (project / "src/main.rs").write_text('fn main(){let r=rake_resources::resources!().unwrap();println!("{}:{}",env!("CARGO_PKG_VERSION"),std::fs::read_to_string(r.path("message.txt").unwrap()).unwrap());}')
        packages = {"deb": [], "rpm": []}
        for version in ("1.0.0", "1.0.1"):
            manifest = f'''[workspace]
[package]
name="rake-distribution-smoke"
version="{version}"
edition="2024"
[dependencies]
rake-resources={{path={json.dumps(str(repository / "crates/rake-resources"))}}}
[build-dependencies]
rake-build={{path={json.dumps(str(repository / "crates/rake-build"))}}}
[package.metadata.rake.bundle]
schema_version=1
identifier="dev.rake.distribution.smoke"
resources=[{{from="message.txt",to="message.txt"}}]
[package.metadata.rake.bundle.linux]
formats=["deb","rpm"]
[package.metadata.rake.bundle.linux.dependencies]
deb=["libc6 (>= 2.39)"]
rpm=["glibc >= 2.39"]
'''
            (project / "Cargo.toml").write_text(manifest)
            run("cargo", "generate-lockfile", "--offline", "--manifest-path", project / "Cargo.toml")
            output = root / version
            result = json.loads(run(cli, "bundle", "--native", "--debug", "--manifest-path", project / "Cargo.toml", "--output", output, "--json"))
            for artifact in result["artifacts"]:
                packages[artifact["format"]].append(output / artifact["path"])
        marker = root / "user-data"
        marker.write_text("preserved")
        try:
            for version, package in zip(("1.0.0", "1.0.1"), packages["deb"]):
                run("sudo", "dpkg", "--install", package)
                assert run("/usr/bin/rake-distribution-smoke", cwd="/").strip() == f"{version}:installed resource"
                assert marker.read_text() == "preserved"
        finally:
            run("sudo", "dpkg", "--remove", "rake-distribution-smoke")
        assert not pathlib.Path("/usr/bin/rake-distribution-smoke").exists()
        rpm_dir = root / "rpm"
        rpm_dir.mkdir()
        for package in packages["rpm"]:
            shutil.copy(package, rpm_dir / package.name)
        run("docker", "run", "--rm", "-v", f"{rpm_dir}:/packages:ro", "fedora:43", "sh", "-ec", '''
for version in 1.0.0 1.0.1; do
  dnf -y install /packages/rake-distribution-smoke-${version}-1.*.rpm
  test "$(cd / && /usr/bin/rake-distribution-smoke)" = "$version:installed resource"
done
dnf -y remove rake-distribution-smoke
test ! -e /usr/bin/rake-distribution-smoke
''')
        print("Debian and Fedora install, launch, upgrade and uninstall passed")


if __name__ == "__main__":
    main()
