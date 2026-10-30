#!/usr/bin/env python3
"""Generated App Installer enrollment and managed upgrade on a disposable runner."""
import argparse
import json
import os
import pathlib
import subprocess
import tempfile
import time
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
    launch_marker = repository / 'target' / f'appinstaller-launch-{os.getpid()}.txt'
    assert not launch_marker.exists(), 'refuse to replace an existing launch marker'
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
                (source / 'src/main.rs').write_text('fn main(){let r=rake_resources::resources!().unwrap();let observed=format!("{}:{}",env!("CARGO_PKG_VERSION"),std::fs::read_to_string(r.path("message.txt").unwrap()).unwrap());std::fs::write(MARKER_PATH,&observed).unwrap();}'.replace('MARKER_PATH', json.dumps(str(launch_marker))))
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
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
[ComImport,Guid("2e941141-7f97-4756-ba1d-9decde894a3d"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IRakeActivation {
    [PreserveSig] int ActivateApplication([MarshalAs(UnmanagedType.LPWStr)] string id,[MarshalAs(UnmanagedType.LPWStr)] string arguments,uint options,out uint processId);
    [PreserveSig] int ActivateForFile([MarshalAs(UnmanagedType.LPWStr)] string id,IntPtr items,[MarshalAs(UnmanagedType.LPWStr)] string verb,out uint processId);
    [PreserveSig] int ActivateForProtocol([MarshalAs(UnmanagedType.LPWStr)] string id,IntPtr items,out uint processId);
}
public static class RakeManagedLaunch {
    [DllImport("ole32.dll")]
    private static extern int CoInitializeEx(IntPtr reserved,uint apartment);
    [DllImport("ole32.dll")]
    private static extern void CoUninitialize();
    [DllImport("ole32.dll")]
    private static extern int CoCreateInstance(ref Guid classId,IntPtr outer,uint context,ref Guid interfaceId,[MarshalAs(UnmanagedType.Interface)] out IRakeActivation manager);
    public static uint Start(string id) {
        int apartment=CoInitializeEx(IntPtr.Zero,2);
        if(apartment<0 && apartment!=unchecked((int)0x80010106)) Marshal.ThrowExceptionForHR(apartment);
        try {
            var classId=new Guid("45ba127d-10a8-46ea-8ab7-56ea9078943c");
            var interfaceId=typeof(IRakeActivation).GUID;
            IRakeActivation manager;
            Marshal.ThrowExceptionForHR(CoCreateInstance(ref classId,IntPtr.Zero,4,ref interfaceId,out manager));
            try { uint processId;Marshal.ThrowExceptionForHR(manager.ActivateApplication(id,null,2,out processId));return processId; }
            finally { Marshal.ReleaseComObject(manager); }
        } finally { if(apartment>=0) CoUninitialize(); }
    }
}
'@
[RakeManagedLaunch]::Start($p.PackageFamilyName+'!App')
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
                    launch_marker.unlink(missing_ok=True)
                    run('powershell.exe', '-NoProfile', '-NonInteractive', '-File', install, url, identifier, version + '.0')
                    deadline = time.monotonic() + 30
                    while not launch_marker.exists():
                        assert time.monotonic() < deadline, 'managed activation did not execute the installed application'
                        time.sleep(0.1)
                    assert launch_marker.read_text() == f'{version}:managed resource'
                print('Generated HTTPS App Installer fresh enrollment, association, managed N+1 upgrade and persistent data passed', flush=True)
    finally:
        powershell("$ErrorActionPreference='Stop'; Get-AppxPackage -Name $args[0] | Remove-AppxPackage", identifier)
        launch_marker.unlink(missing_ok=True)
        if thumbprint:
            powershell(r"""$ErrorActionPreference='Stop'
$store=[System.Security.Cryptography.X509Certificates.X509Store]::new('Root','LocalMachine')
$store.Open('ReadWrite')
try{$matches=$store.Certificates.Find('FindByThumbprint',$args[0],$false);$store.RemoveRange($matches)}finally{$store.Close()}
""", thumbprint)


if __name__ == '__main__':
    main()
