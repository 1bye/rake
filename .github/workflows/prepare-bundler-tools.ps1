$ErrorActionPreference = 'Stop'
# URLs, hashes and layout from tauri-bundler 2.10.4's windows/{msi,nsis}/mod.rs.
$tools = Join-Path $env:LOCALAPPDATA 'tauri'
$downloads = Join-Path $env:RUNNER_TEMP 'tauri-downloads'
New-Item -ItemType Directory -Force $tools, $downloads | Out-Null

function Get-Verified($url, $name, $algorithm, $hash) {
    $file = Join-Path $downloads $name
    & curl.exe --fail --location --retry 8 --retry-all-errors --retry-delay 5 --connect-timeout 30 --max-time 180 --output $file $url
    if ($LASTEXITCODE -ne 0) { throw "Download failed: $url" }
    if ((Get-FileHash $file -Algorithm $algorithm).Hash -ne $hash) {
        throw "Checksum mismatch: $name"
    }
    return $file
}

$wix = Get-Verified 'https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip' 'wix.zip' 'SHA256' '6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31'
Expand-Archive -Path $wix -DestinationPath (Join-Path $tools 'WixTools314') -Force
$nsis = Get-Verified 'https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.13/nsis-3.13.zip' 'nsis.zip' 'SHA1' 'DB14F8AF2EA346B786C6AE19343F61AAF5E09B8E'
Expand-Archive -Path $nsis -DestinationPath $downloads -Force
Move-Item (Join-Path $downloads 'nsis-3.13') (Join-Path $tools 'NSIS')
$plugin = Get-Verified 'https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll' 'nsis_tauri_utils.dll' 'SHA1' '75197FEE3C6A814FE035788D1C34EAD39349B860'
$plugins = Join-Path $tools 'NSIS/Plugins/x86-unicode/additional'
New-Item -ItemType Directory -Force $plugins | Out-Null
Copy-Item $plugin $plugins
Remove-Item -Recurse -Force $downloads
