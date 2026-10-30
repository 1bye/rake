# Desktop distribution

The smallest distributable application is [the desktop example](../examples/distribution).
It uses `rake::view!`, one SVG icon and `rake-build` in build.rs. Application code
imports the Rake facade; installer engines remain CLI dependencies.

```toml
[package.metadata.rake.bundle]
schema_version = 1
name = "My App"
identifier = "com.example.my-app"
icon = "assets/icon.svg"
resources = [{ from = "assets/help.txt", to = "help.txt" }]
```

```rust
// build.rs
fn main() {
    rake_build::build().expect("prepare desktop resources");
}
```

From the workspace:

```sh
cargo run -p rake-cli -- doctor --distribution --package rake-distribution-example --json
cargo run -p rake-cli -- build --native --package rake-distribution-example --json
cargo run -p rake-cli -- bundle --package rake-distribution-example --plan --json
cargo run -p rake-cli -- bundle --package rake-distribution-example --debug --json
```

`build --native` compiles Cargo executables and reports their paths; `bundle`
prepares icons, installed resources, components and a native installation artifact.
Pass `--features feature-one,feature-two` to either command. Required updater
features are merged into bundle builds. `--target desktop|web` keeps its existing
meaning; Rust triples use `--target-triple` and packaging OS selection uses
`--platform`. Cross-host plans are supported; execution requires the selected
native host and tools. Doctor reads metadata and probes PATH without downloads.
Packager-managed tools and Windows SDK discovery get their full native preflight
at execution time.

Schema 1 defaults to DMG on macOS, NSIS on Windows and AppImage on Linux. Select
formats with repeated `--format` flags or platform configuration. `--debug` avoids
a release-profile Cargo build while developing packaging. Ordinary bundle permits
explicit development trust; `--trusted` and `release` require production signing
prerequisites. Use local signing references, never serialized private keys.

## Native settings and dependencies

```toml
[package.metadata.rake.bundle.macos]
formats = ["zip", "dmg"]
info_plist = "packaging/Info.plist"
entitlements = "packaging/app.entitlements"
# identity = "Developer ID Application: ..."
# notary_profile = "existing-keychain-profile"

[package.metadata.rake.bundle.windows]
formats = ["nsis", "msix", "velopack"]
manifest = "packaging/app.manifest"
# certificate_thumbprint = "..."
# timestamp_url = "https://..."

[package.metadata.rake.bundle.windows.msix]
publisher = "CN=Publisher matching the signing certificate"

[package.metadata.rake.bundle.linux]
formats = ["deb", "rpm", "arch", "appimage", "flatpak"]
package_name = "my-app"

[package.metadata.rake.bundle.linux.dependencies]
deb = ["libc6 (>= 2.39)"]
rpm = ["glibc >= 2.39"]
```

Common desktop integration can be configured without writing native metadata:

```toml
[package.metadata.rake.bundle.desktop]
category = "developer_tool"
url_schemes = ["my-app"]
file_associations = [{ extensions = ["mynote"], name = "My Note", mime_type = "application/x-my-note" }]

[package.metadata.rake.bundle.desktop.localizations.fr]
name = "Mes notes"
description = "Documents et notes"

[package.metadata.rake.bundle.macos.privacy]
NSCameraUsageDescription = "Capture an image for a document"
```

Categories, protocols and file types feed macOS bundles, NSIS/MSI registration and
Linux desktop/MIME metadata. MSIX generates managed protocol/file declarations.
Localized labels become macOS `InfoPlist.strings`, Linux desktop/AppStream labels,
and MSIX `resources.pri` through the Windows SDK's `makepri.exe`. Windows localized
OS labels require MSIX; Velopack protocol/file registration needs application
installer callbacks or an explicit backend. Planning rejects unsupported automatic
combinations. Your application handles the resulting native activation events.
A complete custom native metadata file owns those fields; conflicting common
registrations are rejected. Privacy-purpose text merges into the macOS plist and
must match any duplicate custom key.

Identity fields are generated and protected against conflicting overrides.
Windows manifests replace GPUI's staged resource instead of adding a second linker
manifest. Linux desktop and AppStream integration is generated from stable app
identity. Flatpak builds source inside its selected SDK, with vendored Cargo
sources and verified pinned Rust tools. Declare its permissions explicitly.

Use `resources.path("help.txt")`, `resources.helper("ffmpeg")` and
`resources.runtime("gstreamer")` from `rake::resources!()?`. Paths are confined to
the installed payload and work independently of the process working directory.
Native components have exact target-specific local or HTTPS/SHA-256 sources,
license files, and a checked-in `rake-dist.lock`. Directory, framework and selected
GStreamer-plugin recipes share the same input resolver. `--offline-components`
uses only verified cached inputs; `--locked-components` rejects changed inputs.
The media acceptance script demonstrates real FFmpeg and GStreamer deployment.
System runtime dependencies belong to the native package manager, with explicit
per-format package names and ABI baselines.

## Updates and publishing

Updates default off. When enabled, set an HTTPS root and provider public keys:

```toml
[package.metadata.rake.bundle.updates]
enabled = true
channel = "stable"
base_url = "https://downloads.example.com/my-app"

[package.metadata.rake.bundle.updates.public_keys]
sparkle = "packaging/sparkle.pub"
velopack = "packaging/velopack.pub"
appimage = "packaging/appimage.pub"
```

Ownership is bound into the executable and signed installation descriptor.
Sparkle, Velopack and standalone AppImage have provider-specific adapters; APT,
RPM, pacman, Flatpak and MSIX remain managed by their installation channel.
Routed workspaces use their existing save/work/window quit guards for restart.
Standalone roots and custom hosts must provide an explicit approval callback;
staging alone never authorizes their shutdown. Blocking network/SDK operations
belong on an application-owned background worker. Sparkle remains main-thread
owned and its SDK UI is integrated with routed workspace update controls.

See [release configuration](distribution-release.md) for one-command publication,
local signing profiles, required native runner gates and Linux repositories.
See [native acceptance](distribution-native-tests.md) for the repeatable tests.

## Current native evidence and compatibility

| Channel | Established native evidence | Remaining acceptance |
| --- | --- | --- |
| macOS app/ZIP/DMG | Real Rake launch, relocated media, Developer ID/notary/staple, signed Sparkle N→N+1 with Save/Cancel and tamper rejection | Full universal updater matrix and remaining failure scenarios |
| Windows NSIS | Real GPUI build/packaging, metadata/resources | Installed UI/media and conventional installer lifecycle |
| Windows MSIX | Signed package install/upgrade, native identity | App Installer fresh-feed association |
| Windows Velopack | SDK startup and native tool availability | Package/install/update/restart gate |
| Debian/Fedora | Install, unrelated-CWD resources, upgrade/uninstall and ELF baseline tests | Generated repository upgrades and desktop/media matrix |
| Arch/AppImage/Flatpak | Implemented backends and portable contracts | Native makepkg, AppImage N+1, real Flatpak sandbox UI |

`distribution.json` records every artifact, hash, native package version,
components and trust evidence. Tool presence does not imply these native gates
passed. Existing legacy bundle JSON and the Web pipeline continue through their
current compatibility path while native parity is completed. Native Python
packaging retirement is gated on that parity; the Web implementation remains.

`resources.spawn_helper("ffmpeg", ["-version"])?` returns an owned
`rake::resources::HelperProcess`. Its drop/cancel path terminates and reaps the
native process group or Windows Job, including inherited descendants. Keep this
owner with the registered application task. For GStreamer, prepare the command
with `gstreamer_command` and pass it to `HelperProcess::spawn`. Blocking waits
belong on an owned background worker; piped output must be drained concurrently.

Windows native runtime groups also prepare private DLL loader directories before
signing. Set `link_from_app = true` on a runtime used by the main executable;
helper-only runtimes share their component identity with the helper. Separate
helpers can carry different DLL versions with the same basename. Undeclared
imports, architecture mismatches and conflicting versions in one process fail.
`windows.system_dlls` declares explicit DLL prerequisites supplied by the target
system; the adapter never discovers dependencies through the developer's PATH.

## Custom formats and component recipes

Register explicit commands when an enterprise installer or runtime needs a custom
preparation step. Format names are globally unique slugs. A custom format consumes
an already selected built-in format on its native host:

```toml
[[package.metadata.rake.bundle.custom_formats]]
name = "company-macos"
platform = "macos"
input_format = "app"
command = ["python3", "packaging/company.py", "package"]
verify_command = ["python3", "packaging/company.py", "verify"]

[package.metadata.rake.bundle.component_recipes]
codec = ["python3", "packaging/codec.py"]

[[package.metadata.rake.bundle.runtimes]]
name = "codec"
recipe = "codec"
source = "vendor/codec"
options = { plugins = "decoder" }

[package.metadata.rake.bundle.release.custom_required]
aarch64-apple-darwin = ["company-macos"]
```

Commands receive one schema-1 JSON request on stdin, return one JSON response on
stdout, and write diagnostics to stderr. They run without a shell in the package
root, with a deadline and process-tree cancellation. Input is an owned copy;
output is an empty owned directory. Responses are limited to 4 MiB. Backend
responses have `schema_version` and relative `artifacts` paths. Recipe responses
have `schema_version`, a relative `root` directory and every relative native file
in `native_code`; missing native files, wrong architectures and escaping paths fail.
The public Rust contracts are `rake_dist_core::{PackageBackend, ComponentRecipe}`;
`rake_dist::{run_package_backend, run_component_recipe}` checks their outputs.

Custom outputs remain local unless an explicitly configured independent verifier
accepts them. Its request includes application identity/version, target, public
signing-profile references, artifact path and measured SHA-256. Its response must
contain `schema_version = 1`, the same `sha256`, and a nonempty native `policy`.
The verifier must inspect the actual native format, signature, identity and version;
a backend's own success response never establishes those facts. Release requires
the same configured verifier and exact verified bytes, and snapshots selected
inputs before generating feeds or uploading. Extensions execute trusted developer
code; path confinement is an output contract, not an operating-system sandbox.
