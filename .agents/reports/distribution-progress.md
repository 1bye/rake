# Distribution implementation progress

Branch: codex/distribution. Fresh managed worktree; original checkout unchanged.

P0: basic adoption matrix passed on native macOS and Ubuntu; the Windows-only
follow-up passed after native path normalization. Extra local macOS tests passed
custom plist, a relocated linked framework, framework symlinks, and strict
external ad-hoc signature verification. Developer ID identity and the local
1git-bruno notarization profile are available; a read-only history request
validated the profile. No credentials are serialized in project configuration.
The fixture now passed Developer ID signing, Apple notarization and stapled-ticket
validation using the existing local profile. The current rake-foundation binary
was also packaged, relocated and launched through the native UI; the Rake window
rendered its Load JSON control. Cancellation and updater handoff remain open.

P1: added portable raw configuration, validated package plans, generated schema,
read-only Cargo package selection and CLI --plan/--schema. Public-consumer tests
cover documented TOML examples, legacy defaults, package-manager update ownership,
path boundaries, update trust settings and Windows version constraints. CLI tests
exercise structured errors and lockfile preservation. Native availability remains
explicitly pending. Target/format partial overrides now replace arrays and merge public-key maps;
explicit CLI format selection wins. Rust triples must parse and match the OS.
Native version mappings preserve Debian/RPM prerelease ordering, enforce Windows
numeric bounds and require explicit mappings where monotonicity is not automatic.
Nine core, two metadata and three CLI consumer tests pass; focused strict Clippy
and rustdoc pass. The broader CLI tests require local sockets outside the sandbox.

P2: implemented the portable schema-1/explicit-native CLI dispatch, pinned private
engine adapter, Cargo JSON build discovery, staged resource/helper copies,
process-group/Windows Job ownership, deadline/cancellation, per-output locking,
input confinement, inventories and owned transactional directory replacement.
Local CLI end-to-end test packages a real tiny Cargo executable, reads its
resource after relocation, runs it from another directory and refuses an unowned
output without deleting its files. A descendant-held file lock proves cancellation
kills the worker's child. Security tests reject escaping symlinks and protect
unowned output. Focused tests, strict Clippy and rustdoc pass on macOS. New native
executor runs on Windows/Linux remain queued for one consolidated CI run.
Existing legacy/native Python and Web dispatch remain in place during migration.

P3: added rake-build, rake-resources and the platform metadata owners. The
build helper tracks metadata/format changes, rasterizes PNG/SVG icons, and embeds
Windows icon/version resources without adding a second manifest. Custom Windows
manifests replace staged resource #1 with SDK validation/readback and retain
GPUI settings. macOS plist identity conflicts fail instead of changing identity.
Installation descriptors and public keys are written before signing; private
helpers and resources resolve after relocation. Local codec, resource-confinement,
manifest-policy and real consumer packaging tests pass. Strict Clippy/rustdoc pass.
Native mt.exe/PE behavior remains a Windows acceptance gate in the manual workflow.
The existing facade no-default-features Wasm check fails in control/web.rs because
reload::host is GPUI-gated; this unrelated baseline was not changed.

P4: implemented target-specific local and pinned HTTPS components, verified
content cache, bounded ZIP/tar.gz/tar.xz extraction, component lock records,
license copying, private native helpers, runtime/directory/GStreamer/framework
recipes, architecture inspection including universal Mach-O inputs, and macOS
loader normalization against declared staged code. Tests cover verified offline
cache/extraction, corrupt/missing cache, tar-link escape, wrong target sources,
real helper execution after relocation and locked input changes preserving the
previous output. The P5 media suite now runs real FFmpeg and GStreamer after relocation with
103 native files, private scanner/plugin paths, an empty registry, and a clean
child PATH/working directory. It processes a video frame and loads coreelements.
Duplicate library names across components are isolated through declared groups.
This establishes macOS media deployment with these local inputs, not every distro.

P5: implemented inside-out macOS signing, required local identity/profile
preflight, notarization/stapling, App/ZIP/DMG assembly and post-copy verification.
The production executor passed Developer ID/notary/ticket checks on a linked
framework app and ran the final copied binary. A universal pure-Rust app contains
both arm64/x86_64 slices and runs on Apple Silicon; helpers/runtimes must contain
both slices when universal is selected. Added Windows local-store Authenticode
sign/verify, SDK MSIX manifests/assets/assembly, pinned Velopack release-family
packaging and early SDK startup ahead of macro app factories. The SDK disables
auto-apply on startup. Added native Windows SDK/package/signing acceptance suites.
Windows native installation/signing and Velopack packaging remain validation gates.

P6-P9: in progress. Native platform/update acceptance remains required; archive
assembly and portable tests do not establish installation or upgrade support.

CI runs (only one full matrix and one Windows-only retry):
- https://github.com/1bye/rake/actions/runs/41057984993
- https://github.com/1bye/rake/actions/runs/41058997245

Future distribution adoption runs are manual-only and support a single-platform
selection to keep runner cost bounded.

## CI accounting

The inherited Rake push workflow also ran on the two adoption pushes:
41057985075 and 41058896200. Both failed on existing strict rake-protocol lints.
These were additional to the three-platform adoption matrix and Windows-only
retry. Subsequent phase commits remain local to prevent more automatic runs.

Native executor acceptance run 41067548580: Linux passed. Windows core, resource,
codec, manifest-policy and descendant-cancellation tests passed; the CLI package
failed at mt.exe resource replacement. Diagnostic capture and Windows verbatim
path normalization were added. A Windows-only retry is reserved for the next
consolidated native checkpoint. No broad push workflow ran on these pushes.

Windows checkpoint 41077196757 isolated SDK error c10100b7: wildcard root
processorArchitecture is invalid in a definition identity. The platform backend
now maps that field to the actual PE machine, rejects conflicting explicit
architectures, and preserves dependency wildcards. Portable regression tests cover
the PE header and XML rewriting. Canonical native paths now avoid Windows verbatim
prefixes at the engine boundary. Installation/artifact paths normalize separators
before serialization and still reject traversal/drive prefixes; component CPU and
OS binary format are both checked. The macOS media and CLI regressions pass after
these changes. A targeted native retry is still required.

Windows checkpoint 41081006728 passed the manifest stage and exposed the
engine resource iterator dropping the source drive prefix. The worker now runs
from its own directory; request manifest/output paths are resolved before spawn,
and Cargo runs from the selected project so toolchain/config discovery is stable.
The adapter passes only worker-relative resource/icon paths. Local CLI consumer
and strict Clippy checks pass. Independent SDK checks now run even if an earlier
CLI gate fails, collecting their failures in one native checkpoint.

P6 implementation: added the Linux platform owner with native dpkg-deb packages,
RPM generation retaining versioned dependency comparisons, makepkg PKGBUILD and
.SRCINFO output, and Flatpak source builds inside the selected SDK. The Flatpak
compiler is pinned to Rust 1.95.0 official archive SHA-256 values; Cargo sources
are vendored offline, SDK/runtime commits are checked and recorded, and runtime
permissions are explicit. Dedicated source manifests remain available for custom
SDK dependency modules. AppImage continues through the private bundler adapter.
Linux package names/resource directories are stable independently of display names.
Desktop/AppStream metadata, per-format system dependencies, bundled/system runtime
policy, private ELF loader paths and numeric GLIBC baseline verification are wired
into the coordinator. ELF requirements are recorded in distribution.json.

Local P6 verification: 13 core plan tests, six Linux unit/round-trip tests,
component/worker/resource/build tests, four spawned CLI checks, strict Clippy and
rustdoc passed. Native ELF/dpkg tests and Debian/Fedora install/upgrade/uninstall
are added to the manual Linux checkpoint. Arch native execution and Flatpak real
GPUI sandbox execution remain required acceptance work; they are not established
by portable manifest tests. No new Linux Actions run has been started yet.

Windows checkpoint 41086364202 passed spawned CLI packaging/resources and failed
the real GPUI build, then stalled in disposable trust setup. Cancellation requested
to stop wasted runner minutes; diagnostics await job completion. The test trust
step now has an explicit software key provider, noninteractive trust installation
and a two-minute timeout; SDK tests have a five-minute timeout. No production
Windows certificate/private key has been introduced.

P6 native checkpoint 41091230341 passed on Ubuntu 24.04, including real ELF
relocation and baseline rejection, spawned CLI packaging, and Debian/Fedora
installation, relocated launch, upgrade and uninstall. Fedora runs in an isolated
Fedora 43 container. Arch makepkg and the actual Flatpak GPUI sandbox remain open.

P7 first implementation: rake-build embeds immutable update identity, native/Cargo
versions, target, packaging owner, channel, HTTPS root and verification key bytes.
Installed descriptors must match and cannot turn a managed package into an
AppImage. Installer startup captures policy in the consuming application crate;
Velopack auto-application remains disabled. Its Rust SDK startup probe now uses
--veloapp-install 1.0.0 (the earlier --veloapp-version flag is not a Rust SDK hook).

The updater crate now has bounded/cancellable HTTPS transport, authenticated full
AppImage feeds, installed-image ownership verification, verified sibling staging,
exclusive locks, atomic replacement and trusted previous-image recovery. The
pinned Velopack SDK has a signed native-feed transport with contextual signature
and expiry, full-package staging, cancellation/progress and independent cached
package checksum verification. This adds a public updates.public_keys.velopack
reference to configuration; private update-signing keys are never serialized.

Restart handoffs are owned and inert until committed. Router::quit_for_update
uses existing document/history/window/work guards; cancellation discards the
installer and preserves admission. Native workspaces invoke the handoff only at
Ready, and keep a failed restart visible for manual exit. Multiple windows cannot
bypass that failure. Standalone/custom hosts have the same explicit callback
handoff contract. This does not yet establish the OS-quit bridge or Sparkle SDK.

Local evidence: 14 guarded-quit tests including dirty cancellation and failed
restart, seven update tests including signature/channel/expiry/downgrade/recovery
and cached corruption, core tests, native facade consumers, strict Clippy and
rustdoc pass. Real signed N-to-N+1 updater acceptance remains open. A Windows
native Velopack packaging/install/upgrade/uninstall fixture is prepared for the
next consolidated checkpoint. P7 is not complete; Sparkle, OS-initiated quit and
native provider restart acceptance still require implementation/validation.

Windows run 41086364202 was cancelled after trust setup stalled. Both gh and the
connected GitHub tool return BlobNotFound for its job log. A fresh-runner build
bug was independently corrected: ordinary locked Cargo builds may fetch missing
dependencies instead of unconditionally forcing offline mode. This is a concrete
fix, not verified attribution of the unavailable GPUI failure log.

Windows checkpoint 41095152890: core and spawned CLI checks passed. Real GPUI
compilation reached Rake and found existing Unix-only checkpoint filesystem calls
in reload/host.rs; Windows distribution now bypasses that Unix dev-supervisor
checkpoint path without weakening its Unix private-file checks. The Rust SDK
startup call now explicitly names the external ::velopack crate, avoiding the
private adapter module shadowing it on Windows. The pinned native vpk tool
installed successfully. Test certificate creation also succeeded, but adding a
current-user root still prompted and timed out. The disposable runner now imports
only its public test trust into LocalMachine stores, and records the current-user
signing thumbprint before trust setup so cleanup still runs after partial failure.
The manual checkpoint saves dependency caches on failure to avoid rebuilding GPUI
on each targeted follow-up. These fixes require a native follow-up, not an inferred
Windows acceptance result.
