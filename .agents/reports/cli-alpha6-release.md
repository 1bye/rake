# CLI alpha.6 release — 2026-11-14

## Scope and source

Published only `rake-app-cli 0.1.0-alpha.6`; the SDK stays at public
`rake-app 0.1.0-alpha.5` and supporting crates stay at alpha.4.

Release source: `ff27cc0893987995f6fbe373958d5a1722155ea2`, based on merged PR #74
at `fd46166de46f10ff60bfd1478e1e477860b2bf3d`.
Worktree: `/Users/yuriihulyk/Documents/GitHub/rake-worktrees/cli-alpha6`.
Branch: `codex/rake-cli-alpha6`.

This CLI embeds Blaze `0.20261030.1-rake.3`, whose launch-feature dependency
planning fix handles `hdrhistogram` introduced by `--live --cdp` without an app
`devtools` forwarding feature or manual lockfile workaround. Alpha.5 remains
immutable with its rake.2 pin. Alpha.6 also includes main's native build progress.

A CLI-only version bump exposed a scaffold assumption: it would request
nonexistent SDK alpha.6. Registry scaffolding now takes the exact SDK version
from the CLI's tested SDK dependency, including Cargo's normalized published
manifest. Git/path overrides keep their previous behavior. This is the only
additional runtime behavior change in the release; no SDK release was needed.

## Public package

- [rake-app-cli 0.1.0-alpha.6](https://crates.io/crates/rake-app-cli/0.1.0-alpha.6)
- Crate SHA-256: `f965b18faebba90de84e3db2431044a494d4fcfde4e6c8e6a29b17c1ad513f53`
- Pinned Blaze archive SHA-256: `61c03dc72e229167b9cd6ffb8931ec403e0acf872d28bc91c36e3ea2ac03ff30`

Downloaded the public crate and compared it byte for byte with the prepared
archive. Its VCS metadata records the clean release source above. Inspected the
packaged pin, scaffold source and normalized SDK dev-dependency.

## Checks

- CLI unit/integration suites: 407 passed, two existing performance benchmarks ignored.
- Supervisor contracts: 15 passed.
- CLI all-target Clippy with warnings denied, Cargo fmt, focused Rake format/lint,
  publication dry-run and Cargo native package verification passed.
- The scaffold regression failed before the fix (`=0.1.0-alpha.6` instead of
  `=0.1.0-alpha.5`) and passed afterward.
- Seven portable scaffold integration tests passed from the packaged crate.
  The full packaged test attempt exposed an existing local-path test depending
  on sibling `../rake`; it passed in the source checkout. A separate attempt to
  run the packaged unit test was blocked by an existing test-only include of
  sibling `rake-build/assets/default-icon.svg`. Neither sibling exists in a
  standalone CLI crate. These extra packaged-test limitations did not affect
  normal package compilation, installation or the consumer checks below.
- Installed the public crate into a private prefix with Cargo 1.98, `--locked
  --debug`; the installed CLI reports alpha.6. The user's installed CLI was not replaced.
- Generated a fresh app without dependency overrides: registry SDK exactly alpha.5.
  Native and `wasm32-unknown-unknown` library builds both passed with `--locked`.

The whole workspace and live-runtime suite were not repeated for this CLI-only
release. The merged engine-pin change already passed workspace/live checks and
all seven PR #74 CI jobs; current checks cover the changed CLI/scaffolding path.

## Public-install live/CDP smoke

Used the public installed CLI with an empty temporary HOME, no Blaze on PATH,
and no `RAKE_BLAZE` or `BLAZE_ARCHIVE`. The normal installer fetched the public
rake.3 archive from GitHub. Its saved checksum and source metadata matched the pin
and release commit. `rake doctor --json` reported live ready, checksum matching,
and the macro server present; its optional PATH Blaze note correctly says absent.
`wasm-bindgen` was absent from this native test's filtered PATH. The separate
Wasm consumer library build above passed.

Tested the same isolated rdisk fixture as PR #74: base
`cb8f37173a5d2f5f576d0b3e9c49739b1f6711a4`, with the pre-existing local sandbox-safe
asset-layout fix `8875265573d7fad9b3c53ce44bef6a470f7b5ccc`. That fix is not part of
this CLI release. The app used registry SDK alpha.5, only its `live` forwarding
feature, and no `devtools` forwarding workaround. The broad Cargo.lock had no
`hdrhistogram` entry before or after the test and remained byte-identical.

Normal hybrid settings, sandbox on, 64 MiB stack, 16 GiB memory cap, `--bin`:

```sh
rake dev --live --cdp --package rdisk --bin rdisk --no-open --json
```

The app built in Blaze's reported 163.88 s, excluding installer/first-use support
download time. No refusal, macro panic, missing manifest or build-failed event was
reported. CDP DOM/CSS inspection and command discovery worked, as did clicks and
keys. Typed `state-kept` into the SSH form without connecting to a host or invoking
cleanup.

Single smoke observations, not medians or a controlled performance benchmark:

| Edit | Save → apply | Save → observed rendered tree | PID | State |
|---|---:|---:|---:|---|
| `view!` text | 0.739 s | 0.839 s | 69817 | kept |
| `space_label` helper | 0.287 s | 0.355 s | 69817 | kept |

Apply values use runtime timestamps; rendered-tree observations include CDP
polling/RPC delay and are not display scanout timings. A native window screenshot
shows both edited labels and retained input. Keys still worked after both patches.
Both edited source files were restored and patched back into the same process;
file and lockfile hashes match the copied fixture baseline.

Field-change restarts, cross-platform GUI behavior and a new 64-edit soak were
not repeated; this release changes the compiler pin and registry scaffolding,
with the runtime/SDK unchanged.

## Installation, source and cleanup

```sh
cargo +1.98.0 install rake-app-cli --version 0.1.0-alpha.6 --locked --force
```

Existing apps do not need an SDK bump for the Blaze manifest-planning fix.
Live remains experimental and Apple Silicon only. Installing this CLI selects the
new bundle automatically; the old rake.2 release and installations are unchanged.

Evidence: [archives, logs, screenshot and checks](</Users/yuriihulyk/Documents/Codex/2026-11-14/rake-cli-alpha6-release/>).

The test app, supervisor and private compiler were stopped. Task-created targets
and the disposable HOME were removed; public/prepared crate archives, the Blaze
archive, logs, screenshot, fixtures and the privately installed CLI were retained.
Cleanup recovered approximately 7.96 GiB.

The original checkout, installed user CLI/Blaze bundle, fork, engine and system
toolchain were not modified. Cargo used its normal registry cache. Only the CLI
was published to crates.io. No Git branch/tag was pushed, and no GitHub release,
PR or upstream post was created.
