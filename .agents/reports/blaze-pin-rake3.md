# Blaze rake.3 pin and rdisk live/CDP verification — 2026-11-14

## Release

Published [v0.20261030.1-rake.3](https://github.com/1bye/blaze/releases/tag/v0.20261030.1-rake.3)
as an Apple Silicon prerelease from clean fork commit
`a873824d5e9f661f1dca2b6bf889cd1867aa8eb3`.

- Fix: `94016f1d7d6877efb3ff77a9bd898b4689da0ad1`, load manifests from both the broad
  lockfile graph and the launch-specific graph, with each graph's checksums.
- Release docs: `a873824d5`, record the fix and current fork status.
- Upstream base is unchanged: `6560553cc09f00d8d96edc24d85a5e441352ab7c`.
- Fork branch `rake-patches` advanced by fast-forward. Existing rake.1/rake.2 tags
  and releases were not changed. Nothing was posted to upstream repositories.

Archive: `blaze-v0.20261030.1-rake.3-aarch64-apple-darwin.tar.gz`.

SHA-256: `61c03dc72e229167b9cd6ffb8931ec403e0acf872d28bc91c36e3ea2ac03ff30`.

The 121,296,771-byte archive has the same 4,709-entry file list as rake.2.
Changed files are the launcher, engine, rebuilt native macro server, release
manifest and two fork docs. Libraries, bundled Rust source library, licenses and
`blaze-rustc` match the previous archive. The manifest records the expected base,
source commit, version and file hashes. Only the archive, its `.sha256` and
`SHA256SUMS` were uploaded; no Rust-source archive was published.

Built locally with the existing `rake/build_release.py` recipe and pinned
nightly-2026-10-27 compiler/development libraries, in separate output directories.
The engine compiled successfully; the first Rust-source download was reset by the
peer. Retrying the same source and verifying its pinned SHA-256 succeeded, then
the unchanged recipe built the native server and archive.

Creating the release tag also triggered workflow run `43575157504`. It was a
duplicate of this verified local build, so it was cancelled during archive build,
before its publish step. Public assets were not replaced.

## Compiler regression

All nine cases in `harness/metadata_launch_features.py` passed against pinned Cargo:
fresh/existing locks, repeated feature toggles, program selection, complete
package/feature/dependency graphs, outputs `0` and `7`, and stale `--locked` failure.

The packaged engine independently built the direct-feature fixture with sandbox
on and the normal hybrid native macros. Its executable prints `7`, matching Cargo;
the broad lockfile remains byte-for-byte unchanged. No app-level `devtools`
forwarding feature is present.

## rdisk fixture and Cargo reference

Tested an isolated copy of rdisk base
`cb8f37173a5d2f5f576d0b3e9c49739b1f6711a4`, with public `rake-app =0.1.0-alpha.5`
and `rake-build =0.1.0-alpha.4`. The original checkout was not edited by this task.

The copy retains the previously proven local asset-layout fix
`8875265573d7fad9b3c53ce44bef6a470f7b5ccc`: assets are crate-local, and `build.rs`
does not stage files into the source tree. This separate rdisk fix is **not**
included in Blaze or this Rake PR and was not pushed. An app still writing bundle
assets into its source tree must address that independently; sandboxing stays on.

The app exposes only `live = ["rake/live"]`, with no `devtools` feature or direct
`hdrhistogram` dependency. Its normal broad lockfile omits `hdrhistogram`.
Cargo resolves it only with `rake/live,rake/devtools`, leaving the lock unchanged.
This preserves the failure condition instead of avoiding the planner bug.

The same copy built with Cargo and those launch features, launched and returned
its expected inspection tree without the former boundary-cache panic. The first
reference launch omitted creation of the disposable RDISK_HOME directory; the
fixture was corrected and rerun successfully without a product code change.

## Public release install and live/CDP run

Used the CLI built from `codex/blaze-pin-rake3`, with a clean temporary HOME, no
Blaze on PATH and no `RAKE_BLAZE` or `BLAZE_ARCHIVE` override:

```sh
/path/to/new/rake dev --live --cdp --package rdisk --bin rdisk --no-open --json
```

The normal installer downloaded the public release URL. The saved download's
SHA-256 matches the pin and GitHub's asset digest; installed source metadata
matches the release commit. `rake doctor --json` reports rake.3 live ready, all
checksums matching and the fixed macro server present.

The app used the normal Rake hybrid settings, sandbox on, a 64 MiB Rust stack and
16 GiB engine memory cap. No rustc fallback for app functions was enabled.
Blaze reported 213.79 s for the initial build, compiling 220,198 functions in 409
crates. This is the logged build time, not a first-install wall-time measurement.
No refusal, macro-expansion failure or build-failed event was reported.

CDP DOM/CSS inspection, command discovery, native clicks and key input worked.
The client opened the SSH form and typed `state-kept`, without connecting to SSH
or invoking cleanup. Three macro text edits and three helper edits retained PID
89878 and the typed state throughout:

| Edit | Trials | Median save → apply | Median save → observed rendered tree |
|---|---:|---:|---:|
| `view!` text | 3 | 0.930 s | 1.022 s |
| `space_label` helper | 3 | 0.356 s | 0.443 s |

Apply times use runtime timestamps. Render observations poll CDP, including its
RPC/polling delay. Workspace checks ran concurrently; these are functional smoke
measurements, not a controlled behind-editor or photon-level latency benchmark.

Keys still worked after patch six. Both source files were restored to the tested
base, and the original labels patched back into the same process with state kept.
The broad Cargo.lock remained unchanged after the complete run. A native window
screenshot visibly confirms both edited labels and the retained input.

## Rake checks, scope and cleanup

- Workspace with `rake-app/live`: 1,044 passed, four existing ignored, no failures.
- Supervisor contracts: 15 passed. Runtime and installer checks are in the workspace
  suite, including wrong-checksum rejection.
- CLI all-target Clippy with denied warnings, Cargo fmt, and focused Rake format/lint passed.
- rdisk's prescribed `scripts/check.sh`: passed (fmt, all-target/all-feature check,
  denied-warning Clippy, 54 tests, denied-warning docs).

Rake's changes are the pin, installer fixture version, docs and this report. No
SDK/runtime/compiler-interface code changed. The public alpha.5 CLI is immutable
and still embeds rake.2; users need a source build or a subsequent CLI release
for the new default pin. This task does not publish another CLI/SDK version.

Protected manifest/lockfile/binary hashes match the recorded pre-task snapshot.
Concurrent edits in the original rdisk UI were observed during final comparison;
the tested fixture uses the explicitly recorded base, and no test edits were
copied back. The original engine, installed Blaze and system toolchain were not
modified. The test app, supervisor and private compiler processes were stopped.

Task-created build/cache trees were removed, recovering approximately 15.22 GiB.
Archives, checksums, logs, fixture sources, screenshots and a CLI with the new pin
remain outside the repository at:

`/Users/yuriihulyk/Documents/Codex/2026-11-14/blaze-rake3-release/`.

Field-change restarts and Linux were not repeated in this compiler-metadata fix.
