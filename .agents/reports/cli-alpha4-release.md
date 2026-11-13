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

No packages have been uploaded yet.

Raw evidence: `/Users/yuriihulyk/Documents/Codex/2026-11-13/rake-cli-release/`.
The original checkout, installed Blaze bundle, fork and system toolchains are unchanged.
