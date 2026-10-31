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
# signing_identity = "Developer ID Application: ..."
# notary_profile = "existing-keychain-profile"

[package.metadata.rake.bundle.windows]
formats = ["nsis", "msix", "velopack"]
manifest = "packaging/app.manifest"
# certificate_thumbprint = "..."
# timestamp_url = "http://timestamp.digicert.com"

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
OS labels require MSIX. Velopack automatically registers protocols and file types
per user through its install/update/uninstall callbacks, embedded by `rake-build`
and wired by `#[rake::app]` or `rake::early_startup!()`. No application callbacks or
registry scripts are needed. Rake remembers its owned declarations and reconciles them on both SDK updates
and Setup replacement, including removal of every declaration in a new version. Removal checks both application ownership and executable command, and
leaves other handlers intact. File types also appear in Open With; existing
defaults and Windows UserChoice settings are preserved. Protocol keys already
owned by another per-user application are rejected. Portable Velopack packages
do not register handlers. Planning rejects unsupported automatic combinations. Your application handles the resulting native activation events.
A complete custom native metadata file owns those fields; conflicting common
registrations are rejected. Privacy-purpose text merges into the macOS plist and
must match any duplicate custom key.

Identity fields are generated and protected against conflicting overrides.
Windows manifests replace GPUI's staged resource instead of adding a second linker
manifest. Linux desktop and AppStream integration is generated from stable app
identity. Flatpak builds source inside its selected SDK, with vendored Cargo
sources and verified pinned Rust tools. Its explicit Rust target overrides any
workspace default target. Declare permissions explicitly. `linux.dependencies.flatpak`
and system-runtime Flatpak entries contain `pkg-config` requirements (for example
`gstreamer-1.0 >= 1.24`), checked inside the selected SDK before compiling. Use a
custom SDK manifest with dependency modules when that SDK does not supply them.

Use `resources.path("help.txt")`, `resources.helper("ffmpeg")` and
`resources.runtime("gstreamer")` from `rake::resources!()?`. Paths are confined to
the installed payload and work independently of the process working directory.
Bundle a prepared dependency tree in one place. These inputs are packaged into the
app; end users do not install development SDKs or packagers:

```toml
[package.metadata.rake.bundle.dependencies.ffmpeg]
path = "vendor/ffmpeg/{target}"
executables = { ffmpeg = "bin/ffmpeg{exe}", ffprobe = "bin/ffprobe{exe}" }
licenses = ["LICENSE"]
resources = { "ffmpeg/presets" = "share/presets" }
# Omit for a self-contained executable:
libraries = { path = "lib" }
```

`{target}` uses the explicit Rust target, and `{exe}` becomes `.exe` on Windows
and empty elsewhere. Paths inside the dependency are relative to `path`.
Resource maps use installed destination = source. Libraries use the existing
`directory` recipe by default; `recipe = "framework"` or `"gstreamer"`, `plugins`
and `link_from_app` are available. Helpers keep their declared names; the runtime
is available as `resources.runtime("ffmpeg-runtime")`. Target and format overrides
merge dependency entries by name, replacing a whole entry. This convenience
configuration lowers into the same confined inputs, component lock, native
closure checks, signing and installed resource descriptor as explicit components.
Name collisions and traversal fail during planning. The lower-level `components`,
`sidecars` and `runtimes` remain available for pinned downloads and custom recipes.

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

Rake can generate update keys and sign feeds directly from protected files,
environment references or the macOS Keychain. See [release signing](distribution-release.md#update-metadata-and-local-signing-profiles).

Ownership is bound into the executable and signed installation descriptor.
Sparkle, Velopack and standalone AppImage have provider-specific adapters; APT,
RPM, pacman, Flatpak and MSIX remain managed by their installation channel.
Routed workspaces use their existing save/work/window quit guards for restart.
Standalone roots and custom hosts must provide an explicit approval callback;
staging alone never authorizes their shutdown. Blocking network/SDK operations
belong on an application-owned background worker. Sparkle remains main-thread
owned and its SDK UI is integrated with routed workspace update controls.

### In-app update state

Native `App` instances register one shared `rake::updates::UpdateController` when
installed policy is available. The application owns its foreground driver and
background work. No periodic checks or downloads start automatically. Routed
workspace chrome shows availability/progress and offers check, download/cancel,
and guarded restart actions through the same controller on each platform.

Custom views resolve the same service through `page.services()` or root `Services`:

```rust
use rake::updates::{UpdateController, UpdateState};

// Inside a mounted view with access to Services:
let updates = services.get::<UpdateController>()?;
let label = match updates.state() {
    UpdateState::Available(info) => format!("Update {} available", info.version),
    UpdateState::ReadyToRestart(_) => "Restart to update".into(),
    _ => "Check for updates".into(),
};
```

Keep a view-owned task awaiting `updates.changed(snapshot.revision)` and notify
the view on each new snapshot. Registering the listener before reading the state
prevents missed events. Controller clones share policy, selection and staging;
one operation owns the provider at a time. `cancel()` requests bounded cleanup;
`Cancelling` prevents a new download from racing that cleanup. Dropping a driver
cancels work, and late results cannot replace newer state.

`check()` probes eligibility; `download()` starts verified staging. Sparkle checks
use the native information-only API, so an availability badge does not open a
native dialog. Its display version comes from the SDK's eligible appcast item.
`download()` on macOS opens Sparkle's native review/confirmation flow; Sparkle
still owns download, extraction and installation. Windows and AppImage publish
transfer progress directly into the shared model.

`router.restart_for_update(options)` submits ready staging through existing
save/work/window guards. Custom hosts take `updates.restart_handoff(args)` and
commit it only after their own approval; dropping or declining it does not install.
Package-manager builds report `ManagedExternally`, and unbundled development apps
normally have no installed update service.

Custom non-GPUI hosts own a future running `updates.run(execute)`, where `execute`
dispatches `UpdateWork::run` to a blocking executor and returns its owned completion
future. Rake does not create detached threads. Standalone Sparkle hosts additionally
can check availability without a quit guard: Rake initializes an information-only
SDK on the AppKit thread. `can_install()` remains false until an active
`NativeQuitBridge` is attached. Custom hosts then honor native termination and
approval; routed workspaces provide that installation integration automatically.

See [release configuration](distribution-release.md) for one-command publication,
local signing profiles, required native runner gates and Linux repositories.
See [native acceptance](distribution-native-tests.md) for the repeatable tests.

## Native evidence and compatibility

The initial profiles are macOS arm64, Windows x86_64 MSVC and Linux x86_64 GNU.
Linux package-manager acceptance uses Ubuntu 24.04, Fedora 43 and an isolated Arch
container; Flatpak uses pinned Freedesktop 25.08 SDK/runtime commits. Declare an
older glibc baseline explicitly: the ELF check rejects newer observed requirements.
An unset baseline makes no promise about older distributions.

| Channel | Established native evidence |
| --- | --- |
| macOS app/ZIP/DMG | Real minimal app frame/resource; relocated FFmpeg/GStreamer; Developer ID/notary/staple; signed Sparkle N→N+1 with Save/Cancel, cached retry and tamper rejection |
| Windows NSIS/MSI | Signed install, unrelated-CWD resources, private CRT, OS protocol/file registration, upgrade/uninstall; actual installed minimal GPUI frame |
| Windows MSIX | Signed managed install/upgrade/data preservation; SDK-validated identity, protocols/file types and localized PRI; HTTPS App Installer fresh enrollment, channel association, native activation and N+1 upgrade |
| Windows Velopack | Native installer lifecycle and actual signed HTTPS SDK N→N+1 relaunch; tamper, cancel, decline and corrupt-cache rejection |
| Debian/Fedora | Package install/upgrade/uninstall, real ELF relocation/baseline checks, signed APT/RPM fresh-install and repository upgrade |
| Arch | Native makepkg recipe/build/install/upgrade/uninstall and signed pacman repository discovery |
| AppImage | Real signed HTTPS N→N+1 replacement/relaunch with retained history/data; signature, truncation, disk-full, lock, read-only, decline and cancel checks |
| Flatpak | Actual pinned-SDK source build, X11/Wayland sandbox frames/resources and signed repository/ref enrollment plus N+1 upgrade/data preservation |

The acceptance scripts and linked CI evidence are recorded in
[native tests](distribution-native-tests.md). Native tests establish their selected
profiles; additional CPU architectures, older Linux baselines and universal updater
combinations require dedicated native runs. A universal pure-Rust macOS packaging
fixture has separately passed slice checks and Apple Silicon execution.

`distribution.json` records artifact hashes, native versions, components and trust
evidence. Legacy macOS/Linux bundle JSON, archives and unpacked layouts are
preserved by the Rust coordinator. Native Python assembly/signing has been retired;
the Python Web pipeline and development supervisor remain, with native builds
from that supervisor delegated to Rust. Existing Web bundle tests still pass.

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
