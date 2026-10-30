#!/usr/bin/env python3
"""Generated App Installer enrollment and managed upgrade on a disposable runner."""
import argparse
import json
import os
import pathlib
import subprocess
import tempfile
from distribution_test_feed import TestFeed


def run(*args):
    return subprocess.check_output([str(value) for value in args], text=True, timeout=180)


def powershell(script, *arguments):
    with tempfile.TemporaryDirectory(prefix='rake-test-powershell-') as temporary:
        path = pathlib.Path(temporary) / 'action.ps1'
        path.write_text(script)
        return run('powershell.exe', '-NoProfile', '-NonInteractive', '-File', path, *arguments).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=pathlib.Path, required=True)
    args = parser.parse_args()
    assert os.name == 'nt' and os.environ.get('GITHUB_ACTIONS') == 'true', 'requires disposable Windows Actions runner'
    cli = args.cli.resolve()
    repository = pathlib.Path(__file__).resolve().parents[1]
    os.environ['CARGO_TARGET_DIR'] = str(repository / 'target')
    identifier = f'dev.rake.appinstaller{os.getpid()}'
    thumbprint = None
    try:
        with tempfile.TemporaryDirectory(prefix='rake-appinstaller-') as temporary:
            root = pathlib.Path(temporary)
            with TestFeed(root) as feed:
                ca = root / 'public-ca.cer'
                run('openssl', 'x509', '-in', feed.ca, '-outform', 'DER', '-out', ca)
                thumbprint = powershell(r"""$ErrorActionPreference='Stop'
$certificate=[System.Security.Cryptography.X509Certificates.X509Certificate2]::new($args[0])
$store=[System.Security.Cryptography.X509Certificates.X509Store]::new('Root','LocalMachine')
$store.Open('ReadWrite')
try{$store.Add($certificate);$certificate.Thumbprint}finally{$store.Close();$certificate.Dispose()}
""", str(ca))
                source = root / 'source'
                (source / 'src').mkdir(parents=True)
                (source / 'src/main.rs').write_text('fn main(){let r=rake_resources::resources!().unwrap();println!("{}:{}",env!("CARGO_PKG_VERSION"),std::fs::read_to_string(r.path("message.txt").unwrap()).unwrap());}')
                (source / 'build.rs').write_text('fn main(){rake_build::build().unwrap();}')
                (source / 'message.txt').write_text('managed resource')
                install = root / 'install.ps1'
                install.write_text(r'''param([string]$Feed,[string]$Name,[string]$Version)
$ErrorActionPreference='Stop'
Add-AppxPackage -AppInstallerFile $Feed
$p=Get-AppxPackage -Name $Name
if($p.Version.ToString() -ne $Version){throw 'wrong managed package version'}
$getter=Get-Command Get-AppxPackageAutoUpdateSettings
if($getter.Parameters.ContainsKey('PackageFamilyName')){$settings=& $getter -PackageFamilyName $p.PackageFamilyName}
elseif($getter.Parameters.ContainsKey('PackageFullName')){$settings=& $getter -PackageFullName $p.PackageFullName}
else{throw ('Unsupported Appx query parameters: '+($getter.Parameters.Keys -join ', '))}
if(-not $settings.AppInstallerUri){throw ('App Installer association missing: '+($settings|ConvertTo-Json -Depth 8))}
$state=Join-Path $env:LOCALAPPDATA ('Packages\'+$p.PackageFamilyName+'\LocalState')
New-Item -ItemType Directory -Force -Path $state | Out-Null
$marker=Join-Path $state 'rake-acceptance'
if($Version -eq '1.0.0.0'){Set-Content $marker 'preserve me'}elseif((Get-Content $marker) -ne 'preserve me'){throw 'managed upgrade lost user state'}
$p.InstallLocation
''')
                for version in ('1.0.0', '1.0.1'):
                    (source / 'Cargo.toml').write_text(f'''[workspace]
[package]
name="appinstaller-smoke"
version="{version}"
edition="2024"
[dependencies]
rake-resources={{path={json.dumps(str(repository / 'crates/rake-resources'))}}}
[build-dependencies]
rake-build={{path={json.dumps(str(repository / 'crates/rake-build'))}}}
[package.metadata.rake.bundle]
schema_version=1
identifier="{identifier}"
resources=[{{from="message.txt",to="message.txt"}}]
[package.metadata.rake.bundle.windows]
formats=["msix"]
certificate_thumbprint="{os.environ['RAKE_TEST_CERT_THUMBPRINT']}"
timestamp_url="http://timestamp.digicert.com"
[package.metadata.rake.bundle.windows.msix]
publisher="CN=Rake CI Distribution"
[package.metadata.rake.bundle.updates]
enabled=true
base_url={json.dumps(feed.base_url)}
[package.metadata.rake.bundle.release.publisher]
kind="directory"
path={json.dumps(str(feed.site))}
public_url={json.dumps(feed.base_url)}
[package.metadata.rake.bundle.release.required]
x86_64-pc-windows-msvc=["msix"]
''')
                    run('cargo', 'generate-lockfile', '--offline', '--manifest-path', source / 'Cargo.toml')
                    output = root / version
                    run(cli, 'bundle', '--native', '--debug', '--manifest-path', source / 'Cargo.toml', '--output', output, '--json')
                    run(cli, 'release', '--manifest-path', source / 'Cargo.toml', '--from', output, '--json')
                    url = feed.base_url + '/stable/x86_64-pc-windows-msvc/application.appinstaller'
                    location = run('powershell.exe', '-NoProfile', '-NonInteractive', '-File', install, url, identifier, version + '.0').strip().splitlines()[-1]
                    assert run(pathlib.Path(location) / 'appinstaller-smoke.exe').strip() == f'{version}:managed resource'
                print('Generated HTTPS App Installer fresh enrollment, association, managed N+1 upgrade and persistent data passed', flush=True)
    finally:
        powershell("$ErrorActionPreference='Stop'; Get-AppxPackage -Name $args[0] | Remove-AppxPackage", identifier)
        if thumbprint:
            powershell(r"""$ErrorActionPreference='Stop'
$store=[System.Security.Cryptography.X509Certificates.X509Store]::new('Root','LocalMachine')
$store.Open('ReadWrite')
try{$matches=$store.Certificates.Find('FindByThumbprint',$args[0],$false);$store.RemoveRange($matches)}finally{$store.Close()}
""", thumbprint)


if __name__ == '__main__':
    main()
