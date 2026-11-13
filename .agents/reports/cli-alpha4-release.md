# Coordinated CLI and SDK alpha.4 release

## Preparation

Base: `ed7708001a1a5923ac0a06473b8d4489e7bd830e` (main after PR #69).
Branch: `codex/rake-cli-alpha4`; isolated worktree `rake-worktrees/cli-alpha4`.

The owner selected a coordinated crates.io SDK and CLI release. All twenty Rake
packages and the vendored live runtime use `0.1.0-alpha.4`, with matching exact
internal dependency versions. Generated apps continue to use the matching registry
SDK without a Git dependency or a scaffold-specific workaround.

The existing optional Git dependency blocked SDK packaging even with live disabled.
`vendor/blaze-live` is copied from `crates/blaze-live` at upstream
`6560553cc09f00d8d96edc24d85a5e441352ab7c` and published as `rake-blaze-live`.
Its library name stays `blaze_live`, as do its wire protocol, exports and implementation.
The six Rust source files differ from upstream only by rustfmt configuration; this
was verified by formatting the originals and comparing their bytes.
Original authorship, repository, README and MIT/Apache licenses are retained; Rake's
vendoring note is the package README. No engine or toolchain changes are involved.

The root workspace's GPUI patches remain unchanged. Registry consumers use published
GPUI dependencies, as consumers outside the workspace already do. The Notes-specific
GPUI Base editing extensions remain local workspace patches, outside this SDK release.

The `live` feature now enables `gpui`, matching its required UI-thread adapter and
the existing tray feature pattern. A `--no-default-features --features live` check
exposed unresolved GPUI imports before this manifest fix.

## Verification

Initial CLI preflight: 292 tests passed, two ignored.

- Coordinated package dry run: all 21 packages verified, including native SDK and CLI.
- Workspace with `rake-app/live`: 1,042 passed, four ignored, no failures.
- Vendored runtime: two tests passed; six source files verified as formatting-only changes.
- Supervisor: 15 tests passed. Installer tests are included in the CLI/workspace suite.
- SDK `--no-default-features --features live`, packaged fresh browser library, CLI/SDK
  Clippy, Cargo fmt, Rake format/lint and CI spacing passed.
- Vendored runtime has a documented allowance for Clippy's upstream while-let style.
  Custom spacing excludes that package with `--no-deps`, leaving Rake consumers checked.
- Fresh live app: text and helper patches reached the retained render tree; the PID
  and Count: 1 survived both. These were single smoke checks, not a latency benchmark.

The first fake HOME exceeded macOS's Unix socket path limit; the fixture now uses
short `/private/tmp/r4h-jl5qoi0q` as HOME, symlinked to its isolated evidence directory.
The first worker exited without a logged panic/refusal while concurrent release
checks were active. OS logs did not establish why. A retry built and ran successfully.
A subsequent quiet build with an empty private Blaze store succeeded in 136.33 s
(host build-script artifacts and the private compiler download were retained).
Text and helper patches after that build also preserved PID and Count: 1.
`rake doctor --json` reports live ready with matching hashes and the macro server.
The original worker-exit cause remains unproven; no engine patch was made.

## Publication

All 21 packages were published to crates.io on 2026-11-13 from clean source commit
`a4c05c47f5f336277950a23008aae91bc2afc723`. All public archives were downloaded;
their SHA-256 values match the prepared archives and their embedded source commit matches.

- CLI: https://crates.io/crates/rake-app-cli/0.1.0-alpha.4
  SHA-256: `d3c0fac1b9d265098fe97f6fd83175aceb997c312f7918e60b75476b503c7c58`.
- SDK: https://crates.io/crates/rake-app/0.1.0-alpha.4
  SHA-256: `1db430414b4c4a7794ab4eeaed5f403da8ca83ee04d2a94a5af879d41fcdf537`.

The first isolated install command outside the checkout selected the machine's default
Rust 1.95 and correctly refused the package's existing Rust 1.98 minimum. The install
succeeded with `cargo +1.98.0`, without changing the default toolchain.
The CLI and SDK require Rust 1.98 or newer, as before.

The preparation commit is local; no Git branch/tag was pushed and no GitHub release or
PR was created. All package source is available in the published archives.

Raw evidence: `/Users/yuriihulyk/Documents/Codex/2026-11-13/rake-cli-release/`.
The original checkout, installed Blaze bundle, fork and system toolchains are unchanged.


## Published-package smoke test

Installed the public `rake-app-cli` in an isolated prefix using Cargo 1.98
(`--debug --locked`); it reports `rake 0.1.0-alpha.4`.
`rake new alpha4-public` with no dependency override generates
`rake = { package = "rake-app", version = "=0.1.0-alpha.4" }`.
All twelve Rake dependencies in its lockfile come from crates.io, with no Git/path
substitution. The only test-fixture source addition is a small helper for the helper edit.

The generated app started through the published CLI's `rake dev --live --bin` using
the isolated pinned rake.2 installation. Initial build: 69.16 s with a warm shared
store (43,085 functions compiled, 129,977 retained), not a cold-build measurement.
A view text edit and helper body edit both reached the retained rendered tree,
with the same PID 35290 and Count: 1 throughout. The single observed save-to-tree
checks were 0.804 s for text and 0.277 s for helper; these are smoke checks with a
debug-built CLI, not medians or display-scanout measurements. No newly refused
function or proc-macro failure was reported in the successful runs.

The test app, supervisor and isolated daemon were stopped after verification.


## Cleanup and remaining limits

Removed seven temporary build/cache trees after stopping the test processes,
recovering 16.16 GiB. Published crate archives, checksums, logs, source fixtures and
an isolated installed CLI remain in the evidence directory. No target directory
was created in the original checkout.

Live mode remains experimental and Apple Silicon only. The first unproven worker
exit is recorded above; the quiet empty-store build and registry-only live checks
passed afterward. Windows/Linux GUI runs and a browser launch were not repeated
in this release task; browser verification was a Wasm library compile. No GitHub
CI run was started because no branch was pushed.
