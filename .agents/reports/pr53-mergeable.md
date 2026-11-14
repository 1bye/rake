# PR #53: rebase and fresh-app live mode

Worktree: `/Users/yuriihulyk/Documents/GitHub/rake-worktrees/blaze-live-mergeable`.
Branch: `codex/blaze-live-mergeable`.
Code commit: `26dd59e7056dab63ea177e8af71be085d6080baa`. Final documentation commit is reported in the task response.
Original PR head: `514151da77ccf562de4f28202a50776b7f622286`.
Final fetched main: `8ba48c2c1688a0da978f93197f9f35ba78739cf5`.

## Rebase and conflicts

The first rebase used main `986768a8` (30 commits after `85f13c8f`). Main then
advanced through lint-only PR #62 to `8ba48c2c`; the second rebase was clean.
The runtime/example source did not change in that second rebase. All requested
suites were rerun after it. CI spacing then required 417 blank-line insertions
across 13 branch files; a Rust lexer confirmed identical token streams. Format,
Clippy, all CLI tests, live-runtime tests and spacing were rerun after that cleanup.
The whole-workspace and Python supervisor runs precede only this token-identical
formatting. Real GUI/scaffold checks below ran before the lint-only rebase and
formatting; application logic and supervisor tokens are unchanged.

`b99f2988` (frame polling) and `3ce1ac3c` (independent polling) form one
`fix(live): poll independently of frames` commit. It follows the Rust supervisor
commit because its porting notes depend on that commit. No frame-polling version
of the live driver appears in the resulting branch history. Other original
commits remain separate, with their subjects preserved.

Conflicts resolved:

- Keep main's native web/broker/browser services and output capture; layer Blaze
  dispatch, daemon watches and outcomes into the current Rust supervisor.
- Keep Windows process ownership/portable pipes. Make the Blaze-only target
  directory Unix-only; preserve main's platform boundaries.
- Combine main's and the PR's argument, metadata and supervisor tests.
- Retain main's removal of `scripts/dev.py`; keep Python suites' native Rust
  contract mappings instead of restoring the old implementation.
- Preserve default partial-state restart notices, strict `--require-state`, Busy
  retry/sequence fixes and existing Cargo compiler selection from the PR.
- Move the two Blaze acceptance/benchmark scripts off the removed `dev` import
  onto the shared bounded native test client, with an offline IPC regression.

This preserves main's infrastructure rather than restoring earlier Python paths.
The PR's intentional default restart-policy change applies to native Cargo mode
as well as Blaze; only compiler selection remains Cargo by default.

## Fresh lockfile cause and Rake fix

An unmodified `rake new` app first resolved only Taffy **0.15.1**, with no
`blaze-live`, `gpui-fast` or `hdrhistogram` entries. The actual live attempt failed
before compilation: `blaze-live` was not pinned by Cargo.lock, and the engine
reads that Git source only at locked versions. The exact error is preserved in
`before-failure.json`; no lockfile was seeded for that reproduction.

Cargo locks all features declared by the root app, but not every unused feature
of a path dependency. The template now declares:

```toml
[features]
devtools = ["rake/devtools"]
live = ["rake/live"]
gpui-fast = ["rake/gpui-fast"]
```

A genuinely fresh app now locks `blaze-live 0.1.0`, `gpui-fast 0.1.13`,
`hdrhistogram 7.6.0` and **taffy 0.15.0**. The alternate GPUI constraint selects
the compatible Taffy version without adding a separate artificial Taffy dependency.
`--live` already selects only live/devtools through the existing feature logic.
An offline `cargo tree --features devtools` confirms plain dev does not compile
`blaze-live` or `gpui-fast`. Optional sources are resolved/downloaded for the lock;
this does not mean all features are compiled or active.

The template regression fails before the fix and passes after. The opt-in scaffold
build test no longer copies the workspace's Cargo.lock: it starts absent, checks
these packages/versions, then builds native and Wasm with `--locked`.

### Fork follow-up: recorded, not patched

In `crates/blaze-metadata/src/lib.rs`, `units_of` asks for a manifest for every ID
in the resolved graph (`for id in resolve.iter()`; diagnostic near line 421).
Its input manifests come from locked/fetched packages. The planner can therefore
ask for an inactive optional package not present in Cargo's lockfile, including
the reported `no manifest of X was read` cases. It also cannot fetch the unpinned
Git runtime in the original reproduction. A proper engine fix must prune by
selected features or consistently load newly resolved manifests before constructing
units; enabling every optional dependency at runtime is not the fix. No fork or
engine source was changed in this task.

## Real acceptance with the published bundle

The test uses an isolated, byte-identical copy of published `rake.1` through
`RAKE_BLAZE`, with its own runtime, store and targets. No installed files change.
The newly generated app is created with `--rake-path` to this worktree and has no
Cargo.lock at launch; no manual dependency/feature/lockfile changes are made.
Initial startup took 175.88 s with a pre-existing isolated standard-library
cache. This is not a claimed empty-store cold-build measurement.

One observation per edit, seconds; inspection-tree redraw is an upper bound,
not physical display scan-out. First edits include first-edit hydration, and host
load/window occlusion is uncontrolled; these are functional smoke checks, not
medians or a new performance comparison.

| App | Edit | Save → apply | Save → redraw | PID/state |
|---|---|---:|---:|---|
| Fresh hello | text | 1.134 | 1.191 | same PID; count 2 kept |
| Fresh hello | helper | 0.353 | 0.386 | same PID; count 2 kept |
| Fresh hello | post-restart-helper | 0.329 | 0.362 | same PID; count 0 after restart |
| view | text | 1.788 | 1.887 | same PID and retained identity/state |
| view | helper | 0.703 | 0.744 | same PID and retained identity/state |
| notes | text | 4.224 | 4.298 | same PID and retained identity/state |
| notes | helper | 0.541 | 0.607 | same PID and retained identity/state |

The fresh app's field edit changed PID in 3.404 s; the restarted app
was observed in 3.438 s. Count reset from 2 to 0 because Home has no
registered checkpoint. The CLI and visible in-app notice match exactly:

> restarted hello: field change in Home; app state was not preserved for Home (entry-2); registered checkpoints restored

The notice is captured in `cases/hello/fresh/field-notice.png` and in the
`live_restart_notice` runtime event. A subsequent helper patch applies with the
new PID. Plain `rake dev` on the same generated app also launches successfully,
uses `native_restart`/Cargo, responds to clicks and emits no live patch events.
View's count 2 survives both smoke edits; Notes' selected window/tab/entry identity
survives both. Temporary example and scratch Rust edits are restored.

No build-blocking compiler refusal or macro-expansion failure was observed after the fix.
Cargo can print a nonfatal discovery diagnostic for an intentionally invalid
`blaze-manifest/tests/fixtures/bad-toml/Cargo.toml` in the pinned Git checkout;
the commands complete successfully. That dependency fixture was not changed.

## Verification

The first spacing check failed with 695 diagnostics across repeated targets.
`python3 scripts/check-spacing.py --fix` corrected the blank lines and its nested
clean recheck passed. The failure, fix log, original files and token-equivalence
output are all retained; no warning was suppressed.

Checks on main `8ba48c2c`:

- `cargo fmt --all --check`: exit 0.
- `rake format --check`: exit 0.
- `rake lint`: exit 0.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: exit 0.
- `cargo clippy -p rake-app --features live --all-targets --locked -- -D warnings`: exit 0.
- `cargo test --workspace --locked --no-fail-fast`: exit 0.
- `cargo test -p rake-app --features live,view-test-support --lib reload::live --locked`: exit 0.
- `python3 scripts/test-supervisor.py`: exit 0.
- `cargo test -p rake-app-cli --bin rake project_build::blaze_install --locked`: exit 0.
- `python3 scripts/check-spacing.py`: exit 0.

Post-format checks (all exit 0):

- `cargo fmt --all --check`.
- `rake format --check`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- `cargo test -p rake-app-cli --locked --no-fail-fast`.
- `cargo test -p rake-app --features live,view-test-support --lib reload::live --locked`.
- `python3 scripts/check-spacing.py`.

Workspace: **998 passed**, 4 ignored, 145 suite results including
doctests. Live-runtime: **6 passed**. Supervisor Python suite:
**15 passed**, including the native-client regression. Installer: **4 passed**,
including wrong checksums, tampered archives/manifests and confinement.
Fresh scaffold native/Wasm test: **1 passed** (181.5 s).
Spacing uses CI's script, pinned lint library and nightly, with the installed
`cargo-dylint 6.2.0` runner (CI installs 6.3.2). No tooling was installed or updated.
Earlier logs and the before-fix regression failure remain alongside final logs.

The Mac checks do not establish Linux/Windows GUI or Intel live-mode behavior.
No GitHub metadata, PR body or release was changed remotely. Nothing was pushed.
The PR description is a local draft in `pr53-description.md`.

The original checkout and local fork retain their recorded HEAD and working-tree
status. Recorded installed/engine/server binary SHA-256 hashes are unchanged;
`integrity-after.json` lists the exact files checked. No toolchain component was
installed, updated or overwritten.

Raw evidence, commands, screenshots, hashes and per-event JSON:
[local evidence](/Users/yuriihulyk/Documents/Codex/2026-11-12/blaze-live-mergeable/).
