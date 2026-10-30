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
previous output. Media fixture execution will be completed with P5 final signing.
No clean-machine FFmpeg/GStreamer claim is made yet.

P5-P9: in progress. Native platform/update acceptance remains required; archive
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
