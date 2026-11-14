# CLI and SDK alpha.5 release — 2026-11-13

## Source and scope

Based on main after PR #71, `f11d6b0aafd4b4a417c2d9fc6dc9d7b49c9c2cd8`.
Release source: `6873cbdaa4dc077aaa335e72ec9f2ee255ab8d68`.
Isolated worktree: `rake-worktrees/cli-alpha5`, branch `codex/rake-cli-alpha5`.

Published only `rake-app` and `rake-app-cli` as `0.1.0-alpha.5`.
Their unchanged supporting crates remain at exact alpha.4 versions. Generated apps
select the matching registry SDK through the CLI package version.

This distributes the merged preview/live support and virtual-list inspector fix:

- `rake dev --live --preview` accepts the combination, follows body patches and
  reconnects after checkpointed restarts.
- Inspection initializes boundary metadata for virtual-list rows laid out during
  prepaint, avoiding the `captured boundary cache` panic while retaining row identity
  and updated text.

No Blaze engine, bundle pin, toolchain or rdisk asset-layout change is included.
The separate local Blaze manifest-planning and rdisk fixes remain local.
The original checkout and installed Blaze bundle were left untouched.

## Published artifacts

Both public archives were downloaded and compared byte for byte with the prepared
archives. Their embedded VCS metadata records the clean release source commit above,
and the packaged fix source matches the checkout.

| Package | Public version | SHA-256 |
|---|---|---|
| [rake-app](https://crates.io/crates/rake-app/0.1.0-alpha.5) | 0.1.0-alpha.5 | `2de625ae59cc23524fb01b8f60c941f8c590dcd58bb8eeb6fe8f48142b7836fc` |
| [rake-app-cli](https://crates.io/crates/rake-app-cli/0.1.0-alpha.5) | 0.1.0-alpha.5 | `45186786b0b85472bb581abaab15e5d371658f3e153ff2f013b0f141deb00f83` |

## Validation

- Workspace tests with `rake-app/live`: 1,044 passed, four existing ignored, no failures.
- Devtools view/inspection suites: 202 passed, one existing ignored benchmark.
- The virtual-list regression also passed from the packaged SDK against registry
  dependencies, without the workspace GPUI patches.
- Supervisor contracts: 15 passed. Preview contracts: 18 passed.
- Installer and live-runtime tests are covered by the workspace/CLI suites.
- Both packages passed Cargo publication dry runs and native package verification.
- CLI/SDK Clippy with live, devtools and view-test-support, Cargo fmt, Rake format/lint
  and SDK documentation passed.

The release changes only versions, lockfile entries and docs. The merged Rust source
also passed [CI #139](https://github.com/1bye/rake/actions/runs/43511431114) on macOS,
Linux, Windows, portable, browser and spacing jobs. This local release branch was not
pushed and no additional GitHub CI run was started. Windows/Linux GUI and a separate
Wasm app build were not repeated for this metadata-only release preparation.

## Public-package preview/live smoke

Installed the public CLI in an isolated prefix using Cargo 1.98 (`--debug --locked`),
then generated a fresh app with the registry-only alpha.5 SDK. The disposable fixture
adds a helper and registers its counter with the existing checkpoint mechanism.
No SDK dependency override, local compiler override or archive override was used.

A clean temporary HOME downloaded the published pinned Blaze rake.2 bundle and
its first-use Rust support files. `rake doctor --json` reported live ready, matching
checksums and the macro server present. The unrelated `blaze` on PATH was reported
as a note; live mode used the pinned installation. `wasm-bindgen` was absent from
this check's PATH; this was a native browser preview, not a Wasm build.

Blaze reported 258.04 s for the clean app build; this is not a first-install
wall-time measurement. It completed without a reported refusal or macro failure.

The same headless Chrome page stayed open throughout:

| Check | PID | Counter | Result |
|---|---:|---:|---|
| Browser input before edits | 85832 | 3 | Three preview clicks reached the native app |
| Text patch | 85832 | 3 | Updated inspected text and video pixels, 1.146 s |
| Helper patch | 85832 | 3 | Updated inspected text and video pixels, 0.950 s |
| Field change | 86103 | 3 | Strict checkpointed restart; state restored in 5.303 s |
| Further helper patch | 86103 | 3 | Patch and video update worked, 0.419 s |
| Browser input after restart | 86103 | 4 | Click reached the restored app |

The preview reconnected automatically without a page reload. No browser page errors
or capture errors were recorded. These are single smoke observations including
capture/screenshot overhead, not latency medians or native scanout timings.

## Installation and cleanup

```sh
cargo +1.98.0 install rake-app-cli --version 0.1.0-alpha.5 --locked
```

Existing apps must also update their `rake-app` dependency to alpha.5 to receive
the inspector fix; installing the new CLI alone does not replace an app's SDK.
The CLI and SDK retain Rust 1.98 as their minimum version. Live mode remains
experimental and Apple Silicon only.

The app, supervisor and private compiler processes were stopped. Task-created build
and compiler-cache trees were removed, recovering 11.41 GiB. Public/prepared archives,
checksums, logs, screenshots, source fixtures and the private installed CLI remain at:

`/Users/yuriihulyk/Documents/Codex/2026-11-13/rake-cli-alpha5-release/`.

No branch/tag was pushed and no GitHub release or PR was created.
