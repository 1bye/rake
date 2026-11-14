# Published Blaze rake.2 pin — 2026-11-12

## Release and provenance

Published prerelease: https://github.com/1bye/blaze/releases/tag/v0.20261030.1-rake.2

Archive SHA-256: `16036bfbadfb8b590ef38e88f36ebbd727b558e64d6a0fb1eafbace9c76c2026`.

Built locally from clean detached commit `14e7632e9226e3982d725c897b7d02fbca235919`, existing annotated tag object `3b57c9781d9ba57f59377d340b0e7e9edde1d5cc`. The tag was not moved. Its parent is the inference optimization `d72f0bcabf9878938d83fcafc24efb62775de8cd`. The upstream base is `6560553cc09f00d8d96edc24d85a5e441352ab7c`.

Used the tagged CI recipe unchanged: isolated `nightly-2026-10-27` with `rustc-dev`, four-field `build.json` (`version`, `out`, `target_dir`, `toolchain_root` from the pinned rustc sysroot), then `python3 rake/build_release.py build.json`. Default parallelism: 3 Cargo jobs. Engine release compilation: 18m30s; bundled macro-server compilation: 59.61s. Rust compiler revision: `16d1ffac17ad8b739e32483ac2e65fe7e1804977`.

GitHub-hosted macOS arm64 runners were unavailable, so this archive was built locally. `RAKE_BLAZE.json` has the required base and commit. All manifest file hashes and both checksum files passed validation; a fresh download from the public URL matched the upload. Release notes disclose the local build. Only the archive, its `.sha256`, and `SHA256SUMS` were uploaded. `rust-source.tar.gz` is absent from rake.2 assets. rake.1's four assets and both existing tag objects remain unchanged.

## Layout compared with rake.1

| | rake.1 | rake.2 |
|---|---:|---:|
| Compressed bytes | 123,116,916 | 121,281,707 |
| All tar entries, including directories | 4,709 | 4,709 |
| Regular files | 3,737 | 3,737 |

No paths were added or removed. Payload: `blaze`, `blaze-engine`, `blaze-rustc`, the fixed `rust-analyzer-proc-macro-srv`, its two dylibs, `share/blaze/library`, licenses, documentation and `RAKE_BLAZE.json`. All library-source/documentation bytes match rake.1. Changed content: the manifest, four executables and two dylibs. The dylibs' Mach-O section contents are identical; local `codesign` uses 16 KiB signature pages rather than rake.1's 4 KiB, reducing signature sizes. New engine/version metadata and rebuilt executables also change bytes. This is the expected ~121 MB / 4,709-entry layout, not the rejected ~245 MB build. The 3,737 regular-file count is not a competing total-entry count.

## Clean installation and correctness

Prepublication: installed with `BLAZE_ARCHIVE` in a disposable HOME; doctor exited successfully and reported live ready. Its initial harness PATH accidentally hid Cargo together with Blaze; corrected the harness and repeated doctor successfully.

Public check: disposable HOME `/private/tmp/r2h-cr3w0xb1`, initially no `.rake`, no Blaze on PATH, no `RAKE_BLAZE`, no `BLAZE_ARCHIVE`. A fresh app generated with `rake new hello --rake-path .../crates/rake` started through `rake dev --live --bin hello`. No manual manifest/lockfile edits; the generated template already exposes live/devtools/gpui-fast. Doctor reports version rake.2, checksum valid, macro server present, supported platform, ready.

The first real download hit Rake's 180-second timeout after 67,678,235 of 121,281,707 bytes. Raised only the download deadline to 600 seconds and repeated from an uninstalled HOME. The public download and installation then succeeded. SHA-256 verification, extraction limits and pin-only selection remain unchanged. The four offline installer tests include rejection of a wrong checksum and tampered installed files/manifests.

Correctness after live edits, also rendered by plain Cargo from the exact final source:

- `format!("Count: {}", self.count)` → `format!("Count: {} ({})", self.count, self.count)` displays `Count: 2 (2)`.
- `b"abc"` → `b"abcdef"` displays `Bytes: 6 (abcdef)`.
- Inserted a line above `Location::caller()`: live output advances from line 172 to 173; Cargo reports 173.

No refusals or proc-macro panics were observed in the successful Blaze builds. Temporary example edits were restored. The installed user `~/.rake/blaze`, original checkout, existing engine and system toolchain were not modified.

## Three warm trials per edit

Same machine/session and Rake main `cceaea22de01996915cef8edfe1a35ccba2c19f6`. Notes uses identical temporary header/helper fixtures under each bundle. The rake.1 baseline uses a hash-verified isolated copy of the published bundle; rake.2 uses the clean public installation. Hybrid settings, 64 MiB stack and `--bin` are unchanged. No compilation from this task overlaps timed edits. Warm-up/setup/correctness edits are excluded.

| App | Bundle | Text apply | Text redraw | Helper apply | Helper redraw |
|---|---|---:|---:|---:|---:|
| Fresh generated app | rake.2 | 0.449 s | 0.510 s | 0.303 s | 0.337 s |
| Notes | rake.1 | 1.818 s | 1.912 s | 0.483 s | 0.565 s |
| Notes | rake.2 | 0.595 s | 0.660 s | 0.496 s | 0.560 s |

Notes text redraw improves 65.5% in this matched sample. The earlier perf report's ~1.97 s text / ~0.59 s helper are historical references, not a second measurement from this run. The generated app differs from the old rake-view fixture, so its timing is not a direct rake-view before/after comparison.

All 18 timed edits preserved PID and route/window identity. The fresh app's counter remained 2 throughout. Notes checks retained route/window identity; this is a body-patch test, not another checkpoint/restart soak. Foreground bundle was T3 Code before and after every timed edit. No preview frame leases or forced input/redraws were used during timings. Redraw is the first retained render-tree query containing the new text, including query/poll overhead, not physical display scanout. Only three samples per case; other applications remained active and load was recorded per trial.

One-minute load ranges: hello/rake2-clean-public: 29.0–31.1, notes/notes-rake1: 13.9–14.8, notes/notes-rake2: 9.2–9.7.

First text warm-up after process startup (excluded; fresh app also had a helper-setup patch first):

| App | Bundle | Apply | Redraw |
|---|---|---:|---:|
| Fresh generated app | rake.2 | 0.470 s | 0.532 s |
| Notes | rake.1 | 2.336 s | 2.431 s |
| Notes | rake.2 | 1.397 s | 1.495 s |

## Verification and changes

Final post-timeout-change checks: 1,025 workspace tests passed, 4 ignored; 6 live-runtime tests; 30 window tests; 15 supervisor tests; 4 installer tests. Cargo fmt, Rake format/lint, workspace Clippy, live-feature Clippy and CI spacing passed. Ordinary Cargo rendering of the correctness fixture passed.

Rake changes: pin version/URL/checksum, active documentation/test fixture version references, download timeout (180 → 600 seconds), this report. No compiler/toolchain change or new fork commit in this task. Nothing merged; only the requested Rake branch/PR is authorized for push.

Raw evidence and exact commands: [local evidence folder](/Users/yuriihulyk/Documents/Codex/2026-11-12/blaze-rake2-local-build/), including `build.json`, `build.log`, archive comparisons, public verification, doctor JSON, `checks-final/`, per-case JSONL/events/screenshots, and correctness snapshots. Build/toolchain/test artifacts were cleaned only within this task's isolated directories to recover disk space; the tested CLI and release archive were retained.
