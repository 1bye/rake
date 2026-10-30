# Plan for cross platform product distribution

Status: proposed implementation plan. Date: 2026-10-30.

Rake should provide one application configuration and one release workflow for
macOS, Windows and Linux. The implementation covers installation, icons and
metadata, resources, helper executables, native runtimes, signing, publication
and updates. Platform and package-format differences remain explicit behind
small Rust APIs.

Use tauri-bundler as the recommended packaging engine, subject to the adoption
gate below. Keep engine types private. Add dedicated handling for Arch, Flatpak,
MSIX and updater-specific formats. Package creation and update installation are
separate responsibilities.

All commands, crates and new configuration below describe the intended
implementation. Existing bundle commands remain supported during migration.

## 1. Current implementation and migration constraints

The existing implementation provides useful behavior that must survive:

- scripts/bundle.py owns bundle metadata, explicit assets, macOS app assembly,
  icons, dylib relocation/signing, native archives and Web deployment metadata.
- scripts/dev.py owns Cargo selection/builds, release defaults, staging, output
  ownership markers, replacement rollback and bundle JSON output.
- crates/rake-cli/src/project.rs embeds the Python helpers and launches them.
  It uses Unix-specific filesystem and process APIs.
- crates/rake-cli/src/main.rs currently rejects every command on non-Unix hosts.
  Distribution commands need a portable path independently of native control IPC.
- scripts/test-bundle.py checks real archives, relocated resources/dylibs,
  local signing, Web serving and failure rollback. Its Linux layout double
  does not establish Linux execution.
- docs/packaging.md defines current CLI behavior and resource layouts.
- .github/workflows/check.yml runs the existing checks on macOS.
- The workspace pins Rust 1.95, edition 2024 and resolver 3. Preserve these.

Inspect application.rs, the app macro, service composition and guarded quit before
adding update startup or restart handling. Routed quit can approve asynchronous
save decisions; the pinned GPUI OS/Dock termination hook cannot currently veto
termination. An updater must not assume that every OS quit path is guarded.

The pinned gpui_platform enables GPUI's windows-manifest feature on Windows.
GPUI's build script embeds its own resource file. Resolve manifest ownership
before adding application resources; do not enable two competing resource-ID-1
manifests or lose GPUI's DPI/compatibility declarations.

Preserve Web bundling and existing dev/watch behavior throughout this project.
Windows distribution does not require implementing Windows development-control
IPC, CDP or the Python supervisor first. Unsupported commands must identify their
specific missing capability.

## 2. Compare cargo packager and Tauri bundler

Research snapshots: cargo-packager 0.11.8 and tauri-bundler 2.10.4, downloaded
from the official crates.io registry and inspected as Rust libraries.

| Requirement | cargo-packager 0.11.8 | tauri-bundler 2.10.4 |
| --- | --- | --- |
| Generic executable packaging | Library API; CLI can be disabled | Library API; no Tauri GUI runtime dependency in its manifest |
| macOS | App, DMG | App, DMG |
| Windows | NSIS and WiX MSI | NSIS and WiX MSI, confirmed in source |
| Linux | DEB, AppImage, Pacman/PKGBUILD | DEB, RPM, AppImage |
| MSIX and Flatpak | Additional backend required | Additional backend required |
| Assets and helper executables | Resources and external-binary configuration | Resource maps and external-binary configuration |
| macOS metadata/native code | Plist overrides, frameworks, entitlements, signing/notarization paths | Plist overrides, frameworks, custom files, entitlements, signing/notarization paths |
| Runtime updates | Separate cargo-packager-updater | Packaging support for Tauri's updater; a GPUI updater is separate work |
| API/toolchain | Pre-1.0; no rust-version in inspected package manifest | Declares Rust 1.90; explicitly does not promise strict library SemVer |
| Adapter concerns | Unsupported formats may be skipped; notarization credentials may be absent | Disable Tauri marker patching and WebView installation; isolate mutable inputs |

References: [cargo-packager library](https://docs.rs/cargo-packager/0.11.8/cargo_packager/),
[package formats](https://docs.rs/cargo-packager/0.11.8/cargo_packager/enum.PackageFormat.html),
[Tauri bundler library and stability](https://docs.rs/tauri-bundler/2.10.4/tauri_bundler/).

The inspected source snapshots identify these upstream commits:

- cargo-packager: 9e88b74ffb5531acf59c8bc371189a5e82b927d2,
  crates/packager.
- tauri-bundler: 993038c49185307dfc895ab7657d77ca0d5ed7c7,
  crates/tauri-bundler.

The crate-level Tauri format summary omits NSIS; its PackageType enum and Windows
implementation include it. Treat the exact version's source/API as authoritative
when summaries disagree. Its SettingsBuilder exposes binary_patching(false).
Its Windows settings include WebviewInstallMode::Skip. The adapter must set both
explicitly and must not request Tauri updater marker patching.

Source references:
[Tauri settings](https://raw.githubusercontent.com/tauri-apps/tauri/993038c49185307dfc895ab7657d77ca0d5ed7c7/crates/tauri-bundler/src/bundle/settings.rs),
[Tauri bundle execution](https://raw.githubusercontent.com/tauri-apps/tauri/993038c49185307dfc895ab7657d77ca0d5ed7c7/crates/tauri-bundler/src/bundle.rs),
[cargo-packager app assembly](https://github.com/crabnebula-dev/cargo-packager/blob/9e88b74ffb5531acf59c8bc371189a5e82b927d2/crates/packager/src/package/app/mod.rs).

Decision: start with tauri-bundler. Its RPM support covers a required native
format; Arch packaging can use makepkg/PKGBUILD within the Linux backend. Both
candidates leave MSIX and Flatpak work. Cargo-packager remains the fallback
candidate if the adoption gate finds a material integration blocker.

Use one engine in the production implementation. Do not introduce a user-facing
engine selector or ship both libraries for overlapping formats. Keep the selected
version in Cargo.lock and restrict upgrades to reviewed releases with adapter
conformance checks. Do not expose upstream settings or errors as Rake contracts.

### Adoption gate before production dependencies

Build the same tiny Rust consumer through isolated adapters for both candidates
on native macOS, Windows and Linux CI runners. Use a non-GUI fixture first, then
a real Rake application. Record:

1. Rust 1.95 compatibility, selected dependency/features graph and measured
   CLI build/dependency cost. Packaging must not expand the app runtime graph.
2. Successful packages, contents and output paths for common formats.
3. Icon, custom metadata, resource mapping and an architecture-specific helper.
4. A relocated native library and a framework containing relative symlinks.
5. Which native tools are downloaded, their versions/checksums, and offline reuse.
6. Process cancellation/cleanup and concurrent-job isolation.
7. Native signing coverage, notarization behavior and required-credential errors.
8. No mutation of Cargo's original build output or writes outside job staging.
9. No WebView runtime installation or Tauri marker requirement in GPUI packages.
10. Compile-time/resource-resolution behavior on the real Windows app.

Missing release credentials must fail Rake's release preflight even if an
upstream engine would merely warn and skip notarization. Requested formats must
never be silently skipped.

Gate result: a committed comparison report with exact versions, host/target
matrix and selected adapter. Retain tauri-bundler unless a concrete blocker
outweighs its coverage. A fallback decision must include the added RPM work.
Source review alone is not a passing native adoption gate.

## 3. Support contract and target matrix

Model these independently: build host, Rust target triple, package format,
Linux compatibility baseline, installation channel and release channel.

Initial native execution coverage must include macOS arm64, Windows x86_64 MSVC,
and Linux x86_64 GNU. Add macOS x86_64, Windows arm64 and Linux arm64 through
dedicated build/install tests. Universal macOS artifacts require both slices and
matching universal helpers/libraries; they are an explicit target.

| Platform or channel | Intended artifacts | Native build and validation |
| --- | --- | --- |
| macOS direct | App, ZIP, DMG | macOS signing/notary tools; real installed app |
| Windows conventional | NSIS EXE, MSI | Windows toolchain; native install/upgrade/uninstall |
| Windows managed | MSIX and appinstaller | Windows SDK, signing and package-identity checks |
| Windows self-managed updates | Velopack release/installer | vpk packaging and its Rust startup contract |
| Debian family | DEB, optional APT repository | Selected Debian/Ubuntu baselines |
| RPM family | RPM, optional repository | Fedora plus selected RPM compatibility baselines |
| Arch | PKGBUILD, .SRCINFO, native package/repository | Clean Arch build with makepkg |
| Linux portable | AppImage | Oldest supported ABI baseline plus newer distributions |
| Linux sandboxed | Flatpak manifest, repository, flatpakref | Pinned runtime/SDK; sandbox and portal tests |
| Web | Existing site archive | Existing Wasm and deployment contract retained |

Before claiming Linux support, record the oldest supported glibc/runtime baseline
and verify GPUI rendering under Wayland and X11 on the selected distributions.
Build native packages in appropriate containers/chroots; use VMs or actual hosts
for GPU/desktop integration. An archive built on a recent distribution does not
prove compatibility with older ones. Alpine/musl and additional formats become
additional declared profiles rather than implicit claims.

Expose planned, available and validated capabilities distinctly. The release
planner rejects an unavailable target/format instead of substituting another.

## 4. Rust crates and dependency ownership

Create crates when their implementation phase starts; do not add empty crates.

| Crate | Responsibility and dependencies |
| --- | --- |
| rake-dist-core | Portable configuration DTOs, validated plan types, artifact/installation descriptors and typed errors. No GPUI, native tools, credentials or packager dependency. |
| rake-dist | Metadata loading, validation/planning, component resolution/cache, engine adapter, job execution and release publication. Depends on core and platform backends. |
| rake-dist-macos | macOS layout normalization, frameworks/dylibs, plist policy, signable inventory, notarization, final archives. Depends on core. |
| rake-dist-windows | PE resource planning, installer/MSIX/Velopack integration and signing policy. Depends on core. |
| rake-dist-linux | Desktop/AppStream integration, DEB/RPM/Arch/Flatpak/AppImage layouts, dependency mapping and repository production. Depends on core. |
| rake-build | Small build-script helper for generated application resources and link-time metadata. No installer engine or update SDK. |
| rake-resources | Runtime installation descriptor and resource/helper resolution. Depends on the small shared core types. No package tools or GPUI. |
| rake-updates | Optional runtime updater API, owned adapter lifetimes and installation-channel policy. Depends on shared types/resources and only the selected native SDKs. |

Dependency directions:

~~~text
rake-cli -> rake-dist -> rake-dist-{macos,windows,linux} -> rake-dist-core
                   -> private selected-engine adapter
rake-build -> rake-dist-core
rake -> rake-resources -> rake-dist-core
rake -- optional updates feature --> rake-updates -> rake-resources
~~~

TOML parsing, schema generation, artifact downloads and packager dependencies stay
out of the runtime core. The public rake facade reexports runtime functionality;
application authors do not import distribution backends.

Keep engine translation in one private owner in rake-dist; avoid making each OS
crate depend directly on the engine. Platform crates provide validated layouts,
metadata and finalization steps through core types. The selected-engine adapter
turns those into the upstream settings only where the engine is used.

Use enum dispatch for built-in formats/providers. Introduce one backend interface
for actual external/custom formats and recipes, with explicit inputs, outputs,
capabilities and errors. Linux package formats begin as modules, not one crate per
distribution. Native update SDKs initially live in target-gated modules inside
rake-updates; split them when dependency/FFI ownership warrants it.

Public errors must be inspectable and preserve causes. Raw config may be plain
serde structs; validated plans keep invariants private. Native pointers and UI
handles never enter serializable plan types. Sparkle/GPUI UI-thread state may be
local and !Send; background workers receive owned paths/data. Preserve the local
runtime's ownership model rather than forcing everything through Arc/Mutex.

## 5. Simple configuration and compatibility

Keep Cargo.toml as the source of application metadata. The new schema is an
extension of package.metadata.rake.bundle. Name/version/authors/homepage/license
default from the selected Cargo package where meaningful.

Minimal new configuration:

~~~toml
[package.metadata.rake.bundle]
schema_version = 1
name = "Media Studio"
identifier = "com.example.media-studio"
icon = "assets/icon.png"
assets = "assets"
~~~

With updates disabled, schema 1 host defaults are DMG on macOS, NSIS on Windows
and AppImage on Linux. Local bundling may be unsigned/ad-hoc with an explicit
local status. Public release policy requires the selected channel's credentials
and trust checks. Basic bundling does not require a release account.

A project needing several channels adds only the relevant sections:

~~~toml
[package.metadata.rake.bundle]
schema_version = 1
name = "Media Studio"
identifier = "com.example.media-studio"
icon = "assets/icon.png"
assets = "assets"

[package.metadata.rake.bundle.macos]
formats = ["dmg"]
info_plist = "packaging/macos/Info.plist"
entitlements = "packaging/macos/entitlements.plist"

[package.metadata.rake.bundle.windows]
formats = ["msix", "velopack"]
manifest = "packaging/windows/app.manifest"

[package.metadata.rake.bundle.windows.msix]
publisher = "CN=Example Publisher"

[package.metadata.rake.bundle.linux]
formats = ["appimage", "deb", "rpm", "arch", "flatpak"]
desktop_entry = "packaging/linux/app.desktop"
appstream = "packaging/linux/app.metainfo.xml"

[package.metadata.rake.bundle.linux.flatpak]
manifest = "packaging/linux/app.yml"

[package.metadata.rake.bundle.updates]
enabled = true
channel = "stable"
base_url = "https://updates.example.com/media-studio/"

[package.metadata.rake.bundle.updates.public_keys]
sparkle = "packaging/keys/sparkle.pub"
appimage = "packaging/keys/appimage.pub"
~~~

The advanced example deliberately selects update-capable Windows channels.
NSIS/MSI remain selectable conventional installers, with external/manual updates
until a tested compatible installer updater is explicitly added. Velopack is its
own packaging family, not an updater grafted onto an arbitrary MSI.

Configuration rules:

- Missing schema_version reads current fields with legacy behavior and existing
  outputs. Provide a diagnostic showing the explicit schema 1 equivalent.
- Preserve assets = "directory". Add resources = [{from = "...", to = "..."}]
  for mappings; reject destination collisions with assets, helpers and metadata.
- Precedence is documented defaults, Cargo metadata, common Rake settings,
  selected-platform settings, selected-target/format settings, CLI overrides.
  Target-specific component paths can resolve automatically from target triples.
- Object fields merge by defined schema; arrays replace. Do not invent generic
  XML deep-merge rules. Plist overlays have explicit key behavior; Windows uses
  typed defaults plus a complete validated manifest template.
- Identity/version/executable names must agree across generated metadata and
  package identity. Reject conflicting custom overrides with field/path context.
- Preserve required GPUI compatibility/DPI metadata and validate custom Windows
  manifests through the native linker/toolchain.
- Unknown fields and unsupported format/provider combinations are errors.
- Signing secrets are references to Keychain, certificate stores or CI secrets.
  Private keys/passwords are never normal serialized configuration. Redact them
  from errors, Debug output, tool arguments and captured subprocess logs.
- Generate a schema and include all examples in parser/planner tests.
- Icon defaults accept one square PNG. Generate ICNS, ICO and Linux icon sizes;
  allow explicit platform variants. Add SVG input through a pinned renderer
  without requiring developers to generate platform icons manually.
- Categories, localization, URL schemes, file associations and privacy-purpose
  strings have typed common/platform settings and a metadata-file escape hatch.

One-time update setup creates/imports provider keys and records public-key files
and credential references in a dedicated distribution configuration section.
For example, updates.public_keys can map sparkle and appimage to checked-in
public-key files. Private keys stay in the selected credential store. An
updates-enabled plan must resolve its required public keys; it never substitutes
a new key silently. Define and test provider-specific key rotation before
retiring an installed trusted key.

The resolved plan records the selected target, defaults, input hashes, effective
metadata, dependency policy, tools, output files and update owner. Developers can
inspect it before any build, download, signing or upload occurs.

## 6. Resources, helpers and native runtimes

Provide a basic helper path and a richer runtime recipe. Example:

~~~toml
[[package.metadata.rake.bundle.sidecars]]
name = "ffmpeg"
path = "vendor/{target}/ffmpeg{exe}"

[[package.metadata.rake.bundle.runtimes]]
name = "gstreamer"
recipe = "gstreamer"
source = "vendor/{target}/gstreamer"
plugins = ["playback"]
~~~

The placeholders are limited to documented target values: Rust target triple,
target architecture and executable suffix. No shell or environment evaluation.

A component source is either a local tree/file or a pinned archive with an
explicit SHA-256. Named built-in recipes may provide reviewed per-target sources;
there is no unversioned "download latest" during release. Store resolved versions,
source hashes and recipe versions in a checked-in rake-dist.lock beside the
selected package manifest. Preserve Cargo.lock for Rust dependencies.

The resolver owns:

1. Target/architecture and package-family selection.
2. Verified fetch/cache and bounded archive extraction.
3. Explicit runtime/plugin selection and transitive native-library inventory.
4. Private installation paths and metadata/license-notice inventory.
5. Loader-path normalization before signing.
6. A runtime map of logical names to installed paths.

FFmpeg begins as a sidecar plus optional linked libraries. GStreamer requires a
recipe for framework/DLL/SO layout, selected plugins, plugin scanner and registry
cache location. The recipe must handle dynamically loaded plugins explicitly;
static dependency scanning cannot discover every runtime-loaded component.
Write registries/caches into app-owned writable data, not the installed bundle.

On Linux, components can use a bundled or system policy per format. System policy
declares explicit DEB/RPM/Arch dependencies; Flatpak resolves SDK/runtime/modules.
Do not bundle glibc or GPU drivers by blindly following every shared-library edge.

Runtime helpers should resemble the following API; exact signatures are fixed
with external-consumer tests:

~~~rust
let resources = rake::resources::installed()?;
let preset = resources.path("presets/default.json")?;
let ffmpeg = resources.sidecar("ffmpeg")?;
let mut command = ffmpeg.command();
command.arg("-version");
~~~

Resource names are confined relative paths. Helpers resolve app-private binaries
rather than falling back silently to PATH. The same API uses a generated
development layout during rake dev. Relocation and an unrelated working directory
must not break it.

Recipes may prepare a child process environment directly. In-process GStreamer
initialization runs once before worker/UI startup if environment configuration is
required; avoid mutating global environment variables after threads start.
Prefer SDK initialization APIs where possible.

Archive extraction rejects path traversal, device files and escaping links.
Ordinary resources retain the current no-symlink policy. Explicit framework/native
runtime recipes preserve only validated internal relative symlinks, which are
necessary for framework correctness. Never weaken all resource validation just
to accommodate frameworks.

See [GStreamer native deployment](https://gstreamer.freedesktop.org/documentation/deploying/windows.html)
and [macOS relocation and plugins](https://gstreamer.freedesktop.org/documentation/deploying/mac-osx.html).

## 7. Platform assembly and metadata

macOS:

- Extend the existing resource/dylib behavior to explicit frameworks, plugin
  bundles and helper executables. Normalize all native load paths first.
- Reuse engine app assembly with automatic signing disabled where necessary;
  Rake owns the complete native-signable inventory and final verification.
- Sign nested code inside out, then the app. Submit, wait for notarization,
  staple and verify. Create the final ZIP/DMG only after this completes.
- Do not modify a finalized signed bundle. If the engine's one-call DMG path
  cannot preserve this ordering, use the platform DMG tool after Rake finalizes
  the app rather than assembling/signing it a second time.
- Check Developer ID certificate type, hardened-runtime settings, entitlements,
  minimum OS and architecture claims. Keep local/ad-hoc status distinct.
- Enable universal output only when every required native payload has both slices.

Windows:

- Add a build-script integration for ICO, version resources, the merged
  application manifest and install descriptor before linking.
- Reconcile the pinned GPUI manifest first. If its additive feature cannot be
  disabled downstream, use a focused upstream/platform change with native tests;
  do not hide the collision using arbitrary linker flags.
- Use tauri-bundler with WebView installation skipped and marker patching off
  for conventional NSIS/MSI. Check sidecar/DLL/native-runtime signing coverage.
- Use Windows SDK tools for MSIX, generate AppxManifest.xml and appinstaller,
  and verify publisher/package identity consistency.
- Use vpk for the Velopack channel and initialize its Rust SDK before the
  application factory/subscriber/UI starts.
- Sign after executable/resource modification and sign the final installer.
  Authenticode verification must cover the installed binaries and update path.

Linux:

- Generate desktop entries, icon-theme layout, AppStream and file/URL handlers.
- Keep helpers private under the app installation prefix to avoid collisions
  with system tools such as /usr/bin/ffmpeg.
- Build DEB/RPM dependency metadata from the selected system/bundled policies.
- Generate Arch PKGBUILD/.SRCINFO and build with makepkg in a clean environment.
  Recipe generation does not claim publication in the AUR or official repositories.
- Generate Flatpak manifests with pinned SDK/runtime, declared permissions,
  modules and desktop integration. Build in that SDK; a random host-built binary
  copied into Flatpak is not the baseline.
- Verify sandboxed GPUI rendering, filesystem access and portals. First-party
  Flatpak repositories and Flathub submission are separate publication targets.
- AppImage receives a signed installation descriptor and explicit supported
  glibc/kernel/graphics baselines.

References:
[Windows resource definitions](https://learn.microsoft.com/en-us/windows/win32/menurc/resource-definition-statements),
[MakeAppx](https://learn.microsoft.com/en-us/windows/msix/package/create-app-package-with-makeappx-tool),
[Flatpak build workflow](https://docs.flatpak.org/en/latest/first-build.html),
[Arch PKGBUILD](https://man.archlinux.org/man/PKGBUILD.5.en).

## 8. Update ownership and runtime API

An immutable, versioned installation descriptor identifies app/version,
architecture, format, provider, release channel, resource layout and trusted
public update settings. Put it inside the signed package or signed artifact.
Corroborate relevant OS package identity. Environment variables alone must not
turn an APT/Flatpak installation into a self-updating app.

Generate the descriptor before final signing. Build separate cached channel
variants when early startup or executable resources differ, particularly MSIX
and Velopack. Key generated inputs/output directories by target, features and
format; never patch one Cargo binary repeatedly to impersonate incompatible
installation channels.

| Installation | Update owner and Rake behavior |
| --- | --- |
| macOS direct app/DMG | Sparkle; Rake integrates UI-thread lifetime and guarded restart |
| macOS Store | Store-managed capability; no Sparkle replacement |
| Windows MSIX/appinstaller | OS-managed; expose status/actions supported by Windows |
| Windows Velopack | Velopack SDK and its release artifacts |
| Windows NSIS/MSI | External/manual until a compatible provider is explicitly validated |
| DEB installed from APT repository | APT; Rake never overwrites package-owned files |
| RPM repository | Package manager such as DNF or Zypper |
| Arch repository | pacman; AUR recipe workflow is a separate supported external mode |
| Flatpak | Flatpak repository |
| Standalone AppImage | Signed standalone updater adapter after its adoption gate |
| Unbundled development/Web | Unavailable or existing Web deployment behavior |

The application-facing service reports concrete capabilities: self-managed,
externally managed, disabled or unavailable. OS-managed providers must not
pretend to support a generic download/apply operation.

For self-managed updates, model idle, checking, available, downloading, verified,
awaiting application approval, installing, restart-required and failed states.
Preserve provider-specific progress/cancellation capabilities rather than
promising identical operations across every backend.

Downloads can occur while the app is usable. Installing/restarting must request
the existing routed save/disposal decisions and running-job policy. Add a small
host-approved restart handoff before the normal host exits; keep install
responsibility with the adapter/helper. A canceled save decision leaves the old
app running. Standalone roots need an explicit update-preparation callback;
until supplied, stage the update and request manual restart.

Sparkle integration must prove its installation/relaunch delegates can defer
termination through Rake's approval path. The existing non-vetoable GPUI OS quit
hook is a real integration gate. Do not claim all unsaved-data cases are solved
by calling the ordinary OS quit function.

Velopack is a full installer/update channel. Its startup callback belongs ahead
of the app factory, including macro-generated native entry points. Package
creation, installation identity, SDK initialization and update feeds must agree.

AppImage update spike:

- Compare established updater adapters using Rake-produced artifacts.
- cargo-packager-updater 0.2.3 is a candidate, not an accepted dependency:
  source review found a whole-download Vec API and an install path that renames
  the old image before writing the new one. Validate size/cancellation and crash
  consistency rather than assuming those requirements are satisfied.
- Accept an existing backend only after trusted-signature, same-filesystem,
  interrupted-install and recovery tests pass. Otherwise implement the minimal
  full-image staging/verification/atomic-commit adapter using existing HTTP and
  signature libraries; do not implement delta patching or cryptography.
- A successful implementation verifies an authenticated app/version/target
  manifest, streams a bounded download into a sibling temporary file, verifies
  the payload, syncs it and commits without leaving a partial executable.
- Read-only or package-manager-owned locations report external/manual handling;
  never request sudo to replace a system-managed installation.

Trusted signing keys, channels and version comparisons come from the installed
descriptor and authenticated provider metadata. Reject wrong-target/app payloads,
tampering and unintended downgrade/channel switching. Keep compatible data
migrations and recovery policies explicit; reverting a binary does not guarantee
that an older binary can read newer application data.

References:
[Sparkle programmatic setup](https://sparkle-project.org/documentation/programmatic-setup/),
[Sparkle release signing](https://sparkle-project.org/documentation/publishing/),
[Velopack Rust integration](https://docs.velopack.io/getting-started/rust),
[App Installer updates](https://learn.microsoft.com/en-us/windows/msix/app-installer/app-installer-file-overview),
[cargo-packager-updater API](https://docs.rs/cargo-packager-updater/0.2.3/cargo_packager_updater/).

## 9. Execution, artifacts and publication

The orchestrator owns this ordered job:

~~~text
load -> validate -> resolve components/tools -> generate build resources
     -> Cargo build -> stage native payload -> normalize loaders/metadata
     -> package/sign/notarize according to format dependencies
     -> verify final artifacts -> publish immutable files -> promote channel
~~~

Run engine/native packaging in an owned worker process with a per-job working
directory, environment and tool cache. This isolates backend assumptions and
supports reliable cancellation of child tools. Use process groups on Unix and
Job Objects on Windows. Bound logs, download/extraction sizes, timeouts and
parallel job admission. A canceled job must not abandon compiler/signing children.

Operate on staged copies of Cargo output. Preserve current ownership-marker,
overlap and rollback checks. Report release states and retained recovery data;
local atomic output replacement does not make remote publication transactional.

Each final artifact record includes app/Cargo versions, mapped package version,
OS/architecture/baseline, format/provider, file list/hashes, native components,
engine/tool versions, signing/notarization results and update metadata. SHA-256
checksums are integrity records, not substitutes for trusted update signatures.

Use a versioned distribution manifest for multiple artifacts. Preserve current
bundle.json/--json fields for legacy single-output commands. Explicit format
selection has one primary artifact; schema 1 multi-output JSON includes an
artifacts array and documented compatibility fields. Do not silently change the
legacy output to an unrelated shape.

Initial publishers: export-to-directory and GitHub Releases/static HTTPS.
Linux repository producers use existing tools for signed APT, RPM and pacman
metadata and Flatpak repositories. GitHub release assets alone do not form an
APT or pacman repository.

Publish immutable artifacts first, verify their downloadable hashes, then promote
the provider-native feed/repository index. Use conditional writes/locks for
concurrent channel promotion. Partial remote uploads can remain unreferenced and
resumed; a failed target must not advance a shared release channel. Keep previous
versions available according to explicit retention settings.

Generate Sparkle appcasts, MSIX appinstaller, Velopack feeds and Linux repository
metadata from the same artifact records. Do not replace their native protocols
with one invented universal updater feed.

Repository enrollment and signing credentials are one-time setup. Public config
stores identifiers and credential references only. CI runs from release tags or
an explicit release command, with native OS runners and serialized promotion.

See [Arch repository maintenance](https://man.archlinux.org/man/repo-add.8.en)
and [Flatpak repositories](https://docs.flatpak.org/en/latest/repositories.html).

## 10. CLI and developer workflow

Preserve --target desktop|web. Add --platform for OS selection and
--target-triple for Rust target selection; do not overload the existing flag.

~~~sh
rake doctor --distribution --package media-studio
rake bundle --package media-studio --plan --json
rake bundle --package media-studio --format dmg
rake bundle --package media-studio --platform windows --format msix
rake release --package media-studio --channel stable
~~~

These are proposed additions except the existing basic bundle command.

--plan is read-only: it reports unresolved downloads/tools as requirements.
Bundle creates local artifacts. Release applies release trust policy and the
configured publication workflow. An explicit release invocation authorizes its
configured uploads; planning never signs or publishes anything.

The same configuration is consumed by native CI workers. A local cross-host
request produces a precise host/tool requirement unless that combination has
been validated. Do not hide remote-build execution behind an ordinary bundle
command. CI generation can make the native runner matrix easy to adopt.

The new schema uses a small rake-build build script on every platform, with a
no-op or lightweight path where no link-time work is needed. The generated
starter includes it, so the developer does not hand-write platform branches.
Existing applications without that helper retain the legacy packaging path.
A missing helper for configuration requiring embedded resources is a precise
preflight error with the minimal setup instructions. Metadata edits must
trigger rebuilds through correct Cargo rerun-if-changed directives. Manual native
entry points use the same early-startup helper as the generated entry point when
a selected updater requires initialization before the app factory.

### Custom backends and recipes

Expose separate typed PackageBackend and ComponentRecipe extension contracts in
rake-dist-core once the first platform implementations exist. Each receives a
validated plan fragment and owned staging/tool handles, and returns declared
artifacts or a prepared component plus native-code inventory. It cannot claim a
signing/notarization result without the appropriate verifier.

Library consumers register an implementation explicitly. For CLI-only external
formats, support an explicit custom_formats entry with name and argv, for example
name = "company-installer" and command = ["tools/company-packager"]. Pass a
versioned JSON request through stdin and require JSON artifact records on stdout;
logs go to stderr. The request contains staging paths, target metadata and tool
references, never serialized private keys. Validate returned paths/hashes before
publication. Apply the normal worker lifetime and output-confinement contract.
No executable discovery from PATH names inferred from package format, and no
native dynamic-library plugin loader.

Typed platform overrides cover most apps. Hooks, when needed, run explicit argv
at documented pre-signing phases. Any mutation after signing invalidates the
signature status and requires full finalization again. Custom formats remain
unvalidated until their own native acceptance suite passes.

## 11. Ordered implementation phases

| Phase | Scope and affected owners | Required completion evidence |
| --- | --- | --- |
| P0 | Temporary adapters, native prerequisite matrix, engine/update-provider spikes | Source comparison plus native adoption report; selected engine and unresolved capabilities recorded |
| P1 | rake-dist-core, config versioning, schema, plan command and legacy translation | Minimal/advanced examples parse; conflicts and unsupported combinations fail precisely; external consumer tests |
| P2 | Portable CLI build/bundle dispatch, rake-dist executor, staged artifacts; engine integration | Real tiny packages on all three native runners; Windows commands no longer fail due to Unix IPC gating |
| P3 | rake-build, rake-resources, icon generation and metadata overrides | Windows manifest collision resolved; relocated resources/helpers work; icons/version/metadata inspected in installed apps |
| P4 | Component resolver, lockfile/cache, FFmpeg helper and GStreamer recipes | Correct target binaries; clean-machine media fixture; plugin discovery works without developer machine paths |
| P5 | macOS and Windows release finalization, MSIX and Velopack formats | Verified signed packages; notarized/stapled Mac app; real Windows install/upgrade/uninstall |
| P6 | Linux DEB/RPM/Arch/AppImage/Flatpak and dependency policies | Native packages install on supported baselines; Arch recipe builds; Flatpak sandbox runs real Rake UI |
| P7 | rake-updates and application-lifecycle integration | Signed N-to-N+1 updates; dirty-data cancellation; managed package ownership preserved; provider-specific restart tests |
| P8 | Publishers, native feeds and Linux repositories; release CI | Isolated test publication; fresh install discovers an update; failure prevents channel promotion |
| P9 | Examples/docs, compatibility cleanup and Python packaging retirement | Full supported matrix; old bundle/Web contracts preserved; no active duplicate native packaging owner |

Dependencies: P2 follows P0/P1; P3 follows P1/P2; P4 follows P3; P5/P6 use P4.
P7 begins after P0 provider decisions and installs against P5/P6 artifacts.
P8 consumes final artifact/update records from P5-P7. P9 follows the full matrix.

Phases are reviewable changes and may require more than one PR when platforms
have separate prerequisites. Do not mark the project complete after one OS.
Every required format/channel remains on this plan until implemented or explicitly
rescheduled by the project owner.

### Detailed phase work

P0: preserve the existing baseline; add tiny application/helper/library fixtures.
Record actual native tool and runtime requirements. Resolve the Windows GPUI
manifest path and updater handoff feasibility before promising a public API.

P1: add only core/planning crates; keep parsing and validated types distinct.
Define schema, merge/version rules, provider compatibility and package-version
mapping including prereleases and MSIX/MSI limits. Test plans without native tools.

P2: route Rust bundle commands before the Unix-only control dispatcher. Add the
engine adapter, worker/process lifetime, output staging and legacy JSON bridge.
Keep Web/Python dev behavior intact and avoid touching unrelated CDP changes.

P3: add portable runtime paths and link-time preparation. Validate one PNG across
platform icons and optional SVG conversion. Reconcile custom metadata with
generated identity and GPUI behavior through actual native executables.

P4: implement local component sources before pinned remote sources. Add the
lockfile and verified cache. Use a harmless tiny sidecar fixture for regressions,
then an explicitly selected FFmpeg binary and GStreamer plugin fixture for real
media deployment. Generic recipes remain usable for other native dependencies.

P5: complete native trust/finalization and optional universal Mac builds. Use
explicit test credentials for credentialed CI jobs. Ensure Velopack startup runs
before the app factory and demonstrate MSIX package identity/update association.

P6: implement formats and repository metadata incrementally, preserving all
required targets. Build Flatpak within its SDK and verify actual GPUI sandbox
operation. Test both bundled and declared-system native dependency policies.

P7: integrate provider SDKs with owned lifetimes, progress and cancellation.
Exercise unsaved content, multiple windows, active jobs, update decline, reboot
requirements and user-driven manual restart. No provider may bypass save guards
while Rake claims a guarded installation capability.

P8: stage publication on local/test endpoints first. Upload immutable artifacts,
verify accessibility, sign provider metadata, then promote channels with conflict
checks. Test real package-manager upgrade from generated test repositories.

P9: migrate examples, document one-command release and advanced native recipes,
and remove superseded native Python paths only after parity. Retain the current
Web implementation until its equivalent migration passes existing tests.

## 12. Verification matrix and release acceptance

Portable/default checks:

- Format, focused Clippy, unit/component tests, external-consumer tests and
  rustdoc with warnings denied for every new public crate.
- Config parsing, merge precedence, identity conflicts, formats/provider routing,
  prerelease mappings and stable JSON/error contracts.
- Malformed archives/paths, escaping symlinks, component hash mismatch, duplicate
  destinations and partial cache/output failures.
- Inspect dependency graphs: runtime applications must not pull in packagers,
  installer generators or signing tools.
- Explicit native feature/target checks and a Wasm check of the rake facade with
  optional desktop features disabled. --all-targets alone is not a platform matrix.

Native install checks:

- Install N, launch from an unrelated working directory, read a resource, invoke
  the packaged helper and load the native runtime/plugin fixture.
- Verify menus/icons/file associations and custom metadata where applicable.
- Upgrade to N+1; check version, resources, helper and persisted application data.
- Uninstall removes installed files and preserves/removes user data only according
  to the selected documented policy.
- Download/quarantine and Gatekeeper checks on macOS, Windows signature/trust
  checks, Linux package-manager and sandbox launch checks.
- Do not count archive inspection as GPU execution or signing as notarization.

Update/failure checks:

- Invalid/untrusted signature, mismatched app/architecture/channel, downgrade,
  truncated payload, unavailable feed and update cancellation.
- Simulated process interruption during download/staging/install, insufficient
  disk space, read-only installation, competing update process and failed restart.
- Dirty documents/active jobs veto restart without losing their previous state.
- External package-manager channels never enter a self-replacement code path.
- AppImage same-filesystem behavior and recovery are tested under real interruption.
- A partial multi-platform upload leaves current channels unchanged.

Keep ordinary tests independent of live credentials and paid services. Explicit
credentialed/native suites fail clearly when their documented prerequisites are
missing; their absence is not a successful validation result. Use the existing
Cargo runner and project checks; preserve Rust 1.95 and feature boundaries.

Final acceptance: from the minimal configuration, each validated native target can
produce its documented local artifact. With one-time release credentials and a
publisher configured, the same workflow creates trusted downloadable packages,
publishes the correct native feed/repository, and upgrades an installed test app
without losing data. The capability matrix reports the exact scope established.

## 13. Deliverables and boundaries

Deliver: the selected-engine report; versioned configuration/schema/examples;
Rust crate/API contracts; all requested native format backends; resource and
component recipes; native update integrations; publisher/repository support;
native CI/install/upgrade checks; and migration documentation.

Keep app-store/Flathub/AUR submission automation as explicit later publication
adapters, while generating their required metadata/recipes in this project.
Do not imply that producing a package enrolls a publisher in a store or a user's
machine in a repository. Code signing identities, accounts and initial trust
enrollment remain developer setup.

This plan does not create a custom delta updater, a general native-code loader,
or a build farm. Native runners and existing packaging/update tools remain the
execution foundation. Linux support expands through declared tested profiles,
not an unqualified promise of every distribution or libc.

Completion is defined by the native acceptance matrix above, not by crate count
or the presence of configuration fields.

