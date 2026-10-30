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

Windows checkpoint 41099657004 passed real GPUI NSIS packaging, disposable local
trust setup and the pinned Rust updater startup hook. MSIX unpacking passed;
signing/install and Velopack fixture stopped at certificate-subject preflight
because a clean PowerShell process lacked the Cert provider. Subject lookup now
uses the native .NET CurrentUser My store directly. A consolidated retry remains.

P7 native Sparkle: automatic pinned SDK preparation (2.10.1 with verified SHA-256),
framework/license staging, signed-feed plist policy, universal policy propagation,
SDK standard UI and main-thread controller ownership are implemented. An owned
AppKit delegate proxy defers OS termination and preserves GPUI delegate behavior.
Routed workspaces join native update requests to the existing dirty/work/window
quit transaction; cached SDK installations use the same approval path. Native
continuations are consumed once and controller teardown declines retained replies.

Actual macOS N-to-N+1 GUI acceptance passed: signed feed/payload download,
dirty-document Cancel preserving N, cached update retry, Save followed by SDK
automatic relaunch at N+1, persistent user data preservation, and rejection of a
feed modified without resigning. The N+1 startup marker was observed before UI
reattachment. The AppKit terminate probe passed deferred cancellation/approval.
Repeatable fixture preparation and assertions are documented in
scripts/prepare-distribution-sparkle.py and docs/distribution-native-tests.md.
Local portable policy (14), updater (7), guarded quit (14), full SDK Clippy and
native facade updates-sparkle/devtools compilation passed. Linux/Windows N+1
SDK acceptance and the remaining native format gates are still open.

Windows checkpoint 41104892784: MSIX native signing, installation and upgrade now
passed, along with real GPUI NSIS packaging and early startup. Velopack reached
its native tool; pinned vpk rejects --version. The exact 1.2.161 tool was installed
locally and its --help output/pack options inspected. The adapter now probes that
versioned help and disables update checks/interactive console prompts. The first
fixture build exceeded its old two-minute subprocess bound; bundle gets seven
minutes within the existing eight-minute gate. A focused velopack-only mode
skips already-passed GPUI/MSIX/core checks and reuses caches.

P8 implementation: added explicit directory, GitHub draft-release and HTTPS
transactional publishers; all-target/version/format gates, artifact readback,
immutable objects, native feeds and public signing-profile commands. Directory
promotion uses an atomic whole-channel symlink on Unix and retains old generations.
Added SDK-signed Sparkle feed verification against the configured public key,
Velopack native feed sidecars, signed AppImage history, MSIX App Installer metadata
from actual build identity, and Linux repository generation through native GPG,
APT/RPM/pacman/Flatpak tools. Added manual serialized release CI using enrolled
local signing profiles; no real release has been published.

P8 local tests: complete core/dist/Linux suites pass; focused strict Clippy and
rustdoc pass. Real Sparkle 2.10.1 signed-feed fixture verifies and tampering/wrong
keys fail. Publication regressions cover missing target, changed artifact, concurrent
lease, replay/downgrade, unowned output, cancellation and retained immutable files.
A spawned CLI release plan/export/replay check passed in an isolated directory.
Native repository install/upgrade, remote publisher protocol tests and fresh native
updater discovery against generated publications remain acceptance work. These
are implementation commits, not a declaration that all phase gates are complete.

P9 developer workflow implementation: added a portable native Cargo build worker,
read-only distribution doctor/tool requirements, feature forwarding for bundle and
SDK-owned Flatpak builds, and a compiling minimal rake::view! desktop example
with SVG icon/resources/build script. Fixed the existing no-GPUI Wasm reload
module gate; explicit wasm32-unknown-unknown facade compilation now succeeds
(with existing router/web dead-code warnings). Added user documentation that
separates backend implementation from proven native installation capabilities.
Native Python retirement and the legacy JSON bridge remain gated on native parity.

Local P9 verification: six spawned CLI plan/build/doctor/bundle tests passed,
including real compiled feature behavior and installed resources/private helper
execution after relocation. The new minimal desktop example compiles. Strict
Clippy for CLI/dist/example passes. One attempt exhausted local disk while
archiving GPUI; only this task's temporary vpk installation was removed, and the
unchanged check passed on retry. No unrelated checkout or temporary task data was
removed. The focused Windows checkpoint 41106825841 exposed platform-specific
console help output; successful stdout/stderr are now both inspected and ANSI
formatting is stripped before checking an exact version token. Native tool checks
now run before Cargo compilation; another broad native matrix is not needed.

Windows focused checkpoint 41108529192 passed. The native fixture created both
Velopack versions through the Rust CLI, installed them, executed relocated resource
reads, verified user data through N-to-N+1 installer upgrade, and uninstalled.
The early Rust SDK startup hook also passed. This closes native P5 Velopack
packaging/install acceptance; SDK feed-driven updates and guarded restart remain
separate P7/P8 tests. Together with checkpoint 41104892784, native Windows MSIX
signing/install/upgrade and real GPUI NSIS build gates are established.

P8 follow-up: final records now carry the public update policy used by their
format build. Publication rejects destination/channel/key conflicts with that
policy. Source manifests and metadata no longer satisfy required native package
gates; Linux repository input selection excludes recipe/manifest/toolchain files.
RPM verification uses an isolated public key database instead of changing host
trust. APT/RPM native repository upgrade acceptance is added to the Linux fixture.

P7 interruption follow-up: AppImage sibling staging has an owned, bounded journal.
A real subprocess is killed after writing partial staging; the next trusted update
lease removes that recorded partial file while preserving unrelated user files.
Unknown/escaping journals are preserved with an error. Normal decline/cancellation
removes both staging and its journal. Ten updater tests pass; strict Clippy and
rustdoc pass for AppImage/Velopack/native-tests features.

Native acceptance transport can enroll one disposable CA only for an explicit
127.0.0.1 HTTPS root, behind native-tests. Chain and hostname verification remain
enabled; production features retain WebPKI trust. The isolated TLS/signing fixture
passed real HTTPS, hostname verification, untrusted-CA rejection and Ed25519 signing
without changing system trust or reading production keys. A real AppImage N/N+1
publication/update fixture is prepared for the consolidated Linux checkpoint.
The facade now reexports the updates! macro as well as the updates module, and
its no-default-features/updates-appimage consumer compiles.

P6 remaining-gate fixtures: added native Arch makepkg/install/upgrade/uninstall and
signed pacman repository discovery in an isolated official Arch container. Added a
real GPUI Flatpak source build with installed SDK/runtime commit pins and a frame
callback that verifies the installed resource after rendering. Flatpak vendoring
now fetches missing locked sources rather than forcing offline resolution on a
fresh runner. Source compile features and the Flatpak policy stay SDK-owned.
Local Linux package tests and strict dist/Linux/example Clippy pass. The actual
Arch/Flatpak native gates remain pending the single consolidated Linux checkpoint,
which also exercises signed APT/RPM repositories and real AppImage HTTPS updates.

P4 helper lifetime follow-up: rake-resources now exposes HelperProcess and a
confined spawn_helper convenience. A concrete owner retains its Unix process
group/Windows Job and terminates/reaps it on cancel/drop. Stdio handles can move
to readers without detaching the owner. A real child/grandchild regression proves
that dropping the owner releases the descendant's file lock. Resource tests,
strict Clippy/rustdoc and the explicit no-default-features Wasm facade check pass.
Native process dependencies are target-gated out of Wasm; existing router/web
dead-code warnings remain unchanged.

Consolidated Linux checkpoint 41111628107 established: generated signed APT and
RPM repositories support fresh N installation and native N+1 discovery/upgrade;
Arch makepkg/install/upgrade/uninstall and its signed pacman repository pass;
real AppImage artifacts update over a signed HTTPS publication and relaunch N+1,
with installed resources and preserved data. Decline/cancel, competing updater
lock and read-only image rejection also passed through the actual native image.
Transient trust-enrollment diagnostics were followed by successful signed native
repository operations; they are not ignored package-manager failures.

Flatpak stopped before SDK compilation at an empty Cargo config source-table
assignment, which panicked in TOML indexing. Vendor configuration now inserts a
source table explicitly and preserves unrelated Cargo build options. A portable
regression covers the fresh-workspace case. The next checkpoint selects only
Flatpak; the passing repositories/Arch/AppImage gates are not rerun.

CI accounting correction: Flatpak retry 41113849969 was queued before its local
check completed. That compile check caught an unavailable values_mut method in
the pinned TOML map; the retry was immediately cancelled. iter_mut is now used,
and the vendor regression and strict dist-library Clippy both pass before the
replacement retry. This extra cancelled run is recorded rather than omitted.

P4 Windows DLL deployment: added PE import/architecture inspection and closure
preparation from declared runtime groups. Main-process roots require
link_from_app; helpers and runtime subprocesses have isolated loader directories.
Conflicting same-name versions in one process and undeclared imports fail before
signing. System DLL allowances are explicit; redistributable CRT DLLs are not
silently considered OS libraries. Case-insensitive native file inspection is
preserved. Portable core/dist/Windows tests, strict Clippy and rustdoc pass.
A real MSVC DLL/helper relocation test is queued with the next consolidated
Windows SDK gate; this commit does not claim that native test already passed.
