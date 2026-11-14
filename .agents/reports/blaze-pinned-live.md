# Published pinned Blaze — real download acceptance

**The pin now authenticates the GitHub-built release, and a clean install downloads and runs it successfully.** `rake doctor` reports ready. Six text edits and six helper edits preserve the same PID and **Count: 2**. All requested regression suites pass. Both repository commits are local; the existing GitHub release and its assets are unchanged.

## Release and pin

- Repository: **1bye/blaze**; release **`v0.20261030.1-rake.1`**.
- Target: **`aarch64-apple-darwin`**.
- URL: [published Blaze archive](https://github.com/1bye/blaze/releases/download/v0.20261030.1-rake.1/blaze-v0.20261030.1-rake.1-aarch64-apple-darwin.tar.gz).
- **SHA-256: `aec30a92722bed32f7dd5d101520ab5554149339a66ac670a352065865457e10`**; downloaded size **123,116,916 bytes**.
- Authenticated archive manifest: source **`f00630273ec384b0679bf7bf0ae21a29dd17eb3a`**, upstream base **`6560553cc09f00d8d96edc24d85a5e441352ab7c`**, Rust **`16d1ffac17ad8b739e32483ac2e65fe7e1804977`**, 3,736 recorded files.

`crates/rake-cli/blaze-pin.toml` replaces the local-build checksum with the published checksum and removes the unpublished-repository comment. Development documentation now describes the published download. The local archive's old SHA `61688ec70ece2dd1e78193e27fdd58461584e9723e665f719fdab85f7478a0c6` is historical and is not accepted by the current pin.

[Independent downloaded-archive verification](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/download-verification.json). The full upstream-base reconstruction and local-bundle baseline are retained in [the previous report](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/previous-report.md) and [the ancestry evidence](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/ancestry.json).

## Real installation

The existing `~/.rake` was moved aside before the first launch. The benchmark enforces an absent Rake home, removes every PATH directory containing Blaze, keeps private links to Cargo's metadata tools, clears `RAKE_BLAZE`, and runs with **no `BLAZE_ARCHIVE`**. `rake dev --live --manifest-path examples/view/Cargo.toml --bin rake-view --json --no-open` therefore used the production HTTPS downloader. Its stderr records installation from 1bye/blaze; independent hashing of the saved downloaded archive matches the pin. The pinned Rust toolchain cache was reused, so this does not measure a fresh Rust toolchain download.

The app's cold build completed and launched from the downloaded installation. Its first session was stopped before timed edits to correct a window-placement issue. Stage Manager had transformed the counter into a **171×144 thumbnail**, while AppKit still reported cached full-window visibility. The timed session reused the same downloaded compiler and target/store, with the existing **`RAKE_DEV_PREVIEW=1`** flag to keep the window on stage. In the app, this flag only changes native collection behavior. `--no-open` remained set; no preview recording, frame lease, input or redraw command was requested during timed edits. No engine code, toolchain or global Stage Manager preference was changed.

The earlier local installation with the same version but a different checksum remains at `/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/previous-rake-home/blaze/0.20261030.1-rake.1`. The published version remains active in `~/.rake/blaze/0.20261030.1-rake.1/`; the previous older version was restored alongside it. No old installation was silently overwritten.

[Clean-download configuration](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/benchmark.json), [initial installer/build stderr](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/download-trials/supervisor.stderr), [initial launch events](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/download-trials/supervisor.jsonl), [Stage Manager finding](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/stage-manager-setup.json), [timed-session configuration](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/warm-benchmark.json).

## Doctor readiness

The real installed bundle's `rake doctor --json` exited successfully:

```json
{"supported":true,"installed":true,"version":"0.20261030.1-rake.1","checksum_matches":true,"macro_server_present":true,"ready":true}
```

[Full doctor result](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/warm-trials/doctor.json).

## Six-trial warm results

One first edit and one warmup of each kind precede the six measured trials of each kind. Text changes are `Live text trial 201..206` inside `view!`; helper changes are `Helper trial 201..206`. The fixture/render protocol matches the local-bundle benchmark. The published window measured **1536×996**; the earlier local window measured **1510×996** despite the same requested bounds.

All twelve before/after preview records report **occluded**, with PID **92078** unchanged and **Count: 2** retained. Setup clicks occur before timing; after saves the harness sends only diagnostic and tree queries. Redraw means the first completed inspected render containing the new marker, not physical display scan-out.

| Edit | Published save → apply | Published save → redraw | Local-bundle redraw | Difference |
|---|---:|---:|---:|---:|
| `view!` text | **0.885 s** | **1.063 s** | 1.003 s | **+59.5 ms / +5.9%** |
| Helper | **0.438 s** | **0.675 s** | 0.594 s | **+81.3 ms / +13.7%** |

Application is faster than the local-bundle medians (0.959 s text / 0.491 s helper), while observed redraw is slower. Per-trial apply → completed-render medians are **211.846 ms** and **224.392 ms**, versus roughly 44 ms in the earlier local run. These component medians need not sum to total medians.

Blaze phase medians from the published run:

| Phase | Text | Helper |
|---|---:|---:|
| Source apply / expansion and outlines | 60.874 ms | 4.535 ms |
| Roots | 54.528 ms | 46.161 ms |
| Cached-code validation | 490.221 ms | 108.513 ms |
| Link / publication | 1.139 ms | 1.009 ms |

The longer observed redraw interval is after application, rather than a slower text compilation/validation phase: text validation was 490.221 ms here versus 537.166 ms locally. The Stage Manager collection setting, slightly different window width and separate background workload prevent attributing this difference solely to CI-built bytes. On 10 logical CPUs the recorded one-minute load ranges were **7.00–7.52** for text and **7.00–7.16** for helper. No performance fix was applied.

[All trial records](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/warm-trials/trials.json), [medians and deltas](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/comparison.json), [full benchmark result](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/warm-trials/result.json), [window geometry and occlusion](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/warm-window-before.json). Raw per-edit worker logs are stored beside those records, outside the repository.

## Wrong checksum and regressions

- `cargo test --workspace --locked`: **932 passed, 4 ignored**.
- Live-runtime tests: **5 passed**.
- `scripts/test-supervisor.py`: **14 passed**.
- Installer tests: **4 passed**. In particular, **`wrong_checksum_cannot_install`** supplies an incorrect checksum and verifies rejection before any installation directory is created. Archive/installed-file/manifest tampering and path checks also pass.

The wrong-checksum case is an offline fake-archive regression test of the same installer path; the public download and its correct checksum are exercised by the real application run above. Neither the pin nor the published release was weakened for testing.

[Regression commands/results](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/regressions.json), [workspace log](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/workspace.log), [installer log including checksum rejection](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/installer.log), [runtime log](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/runtime.log), [supervisor log](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/supervisor.log). Formatting, lint and final integrity results are retained in [final checks](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/final-checks.json).

## Fork fix and commits

Local fork commit **`6625a86d591c106f92eba1dd62de8dd972c22baf`** narrows both `actions/upload-artifact` and `gh release create` in `.github/workflows/release.yaml` from `dist/*.tar.gz` to **`dist/blaze-v*-aarch64-apple-darwin.tar.gz`**. A filename-match check confirms the Blaze archive matches and `dist/rust-source.tar.gz` does not. Checksum uploads remain. This changes future workflow runs only; **the already-published release and its assets were left as-is**. The workflow was not dispatched.

[Upload-pattern check](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/workflow-check.json), [local commit IDs](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/commits.json), [original-checkout/binary integrity](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-published-release-evidence/integrity-after.json).

The Rake follow-up commit updates the published checksum, report/development docs and the benchmark's optional archive input. Both repositories are clean after their local commits. Nothing is pushed and no new release, repository, PR or upstream post is created.
