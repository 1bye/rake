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

P3-P9: in progress. Native platform/update acceptance remains required; archive
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
