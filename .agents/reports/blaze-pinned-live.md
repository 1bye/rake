# Pinned Blaze — verified upstream history and measured bundle

The local fork now descends from **upstream `v0.20261030.1` (`6560553cc09f00d8d96edc24d85a5e441352ab7c`)**. The rebuilt **`0.20261030.1-rake.1`** bundle installs into a clean Rake home with no Blaze on PATH, builds GPUI, and preserves PID and counter state through twelve measured live edits. Nothing was created, pushed, released or posted on GitHub.

## Upstream provenance

A fresh `--no-checkout` clone of `https://github.com/indexable-inc/blaze` is at `~/Documents/GitHub/blaze-upstream-reference-20261108`, used only as a reference. All requested tags (`v0.20261027.0`, `v0.20261028.0` through `.14`) were compared, then the search was widened: 749 first-parent/tag candidates and 2,271 reachable main ancestors dated October 26 through November 2. Diff sizes use `--no-renames`, upstream candidate → recovered snapshot `2007d7c`.

| Candidate | Changed files | Added lines | Deleted lines |
|---|---:|---:|---:|
| **`v0.20261030.1` / `6560553`** | **30** | **862** | **58** |
| `5cf29cd` | 31 | 879 | 58 |
| `v0.20261030.0` / `de410cf` | 32 | 957 | 75 |
| `dd5a5cf` | 35 | 1,132 | 83 |
| `v0.20261028.14` / `c83e0cd` | 621 | 217,166 | 45,054 |
| `v0.20261028.0` / `500900a` | 5,904 | 2,793,609 | 456,271 |
| `v0.20261027.0` / `5d4f78f` | 10,685 | 3,286,934 | 1,264,917 |

[All requested tag metrics](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/tag-comparison.json), [ancestor ranking](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/all-ancestor-ranking.json), [closest candidates](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/closest-candidates.json), [30-file residual diff](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/snapshot-vs-chosen.diff).

**WASI realpath and cross-file `span_join` are lab changes at this base.** Upstream `6560553` has the unconditional EINVAL readlink stub and the `assert_eq!(first.anchor.file_id, second.anchor.file_id)` assertion. The recovered root already contains their fixes.

The residual also contains eight earlier lab fixes beyond the seven-item list: configured job limits/NUM_JOBS keys; per-proc-macro Wasm graph roots; synchronous bounded Wasm preparation; Wasm inlining; MIR storage-conflict deltas; linker-used data/verbatim symbols; C ABI projected-field normalization; x86 AES keygen assist. The job-limit group includes a missing `profile: None` in a daemon test fixture. These are preserved explicitly as prerequisite commits. They are classified inherited snapshot deltas, not a reconstruction of each author; the x86 AES change is outside the Apple Silicon release target. No other unclassified snapshot source drift remains.

## Reconstructed fork

`~/Documents/GitHub/blaze-rake-fork`, branch `rake-patches`, head **`f00630273ec384b0679bf7bf0ae21a29dd17eb3a`**. `upstream` is indexable-inc/blaze. The old branch is retained locally as **`snapshot-archive`**, tip `0641b76574e18874f97ff2ea0747e102c002149a`. Its root `2007d7c` is not an ancestor of the release branch. The release branch has the same root ancestry as the real upstream base and can be pushed into an actual GitHub fork. Later upstream main has an additional merged root; that does not alter the base ancestry check.

The seven requested fixes, native-server build and release recipe are separate commits, following the eight explicit prerequisites:

```text
ba5fbbe7f fix(build): honor configured job limits
034f87148 fix(macros): root Wasm graphs at the selected macro
a9decd896 fix(macros): bound Wasm preparation to its job slot
0a842453d perf(macros): inline Wasm token helpers
1e5131d46 perf(mir): record storage conflict deltas
d7b1c6b46 fix(link): retain used data and verbatim symbols
ee50e65d2 fix(abi): normalize projected aggregate fields
8074bb540 fix(x86): lower AES keygen assist
984cccbff fix(wasi): support confined package realpath
435ff9b80 fix(macros): decline joins across source files
99616c885 fix(codegen): give std its own symbol namespace
1eca44c43 fix(attrs): resolve module paths in symbol names
36e9c3b72 fix(toolchain): give macros canonical Cargo paths
4f06a6337 fix(probes): use real rustc in hybrid host mode
09711e19b fix(neon): lower the GPUI dependency intrinsics
e95ec77c2 build(macros): bundle a matching fixed native server
f00630273 build: prepare pinned Apple Silicon releases
```

`git diff snapshot-archive HEAD -- crates vendor library harness` is empty. Reconstructed engine/compiler inputs match the previously tested stack byte for byte; the remaining whole-tree differences are the corrected provenance docs, upstream-base file and archive manifest's base value. [Ancestry proof](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/ancestry.json), [source parity](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/reconstructed-source-diff.json), [commit list](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/fork-commits.txt).

`RAKE_FORK.md` now documents `git fetch upstream --tags` and `git rebase --onto <new-tag> "$(cat rake/upstream-base)" rake-patches`, updating the base/server pins and running acceptance before any new release. Only the two Apple Silicon fork release workflows remain active.

## Rebuilt archive and installation

- Version: **`0.20261030.1-rake.1`**, target **`aarch64-apple-darwin`**.
- Built commit: **`f00630273ec384b0679bf7bf0ae21a29dd17eb3a`**.
- SHA-256: **`61688ec70ece2dd1e78193e27fdd58461584e9723e665f719fdab85f7478a0c6`**.
- [Archive](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/release/blaze-v0.20261030.1-rake.1-aarch64-apple-darwin.tar.gz), [checksums](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/release/SHA256SUMS), [build configuration](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/build.json), [build log](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/build.log).

The standard archive root includes the three compiler binaries, 3,724 library source files, licenses, fixed native macro server, matching loader-relative server libraries and authenticated `RAKE_BLAZE.json`. Its base is the verified upstream commit. The server matches pinned Rust **`16d1ffac17ad8b739e32483ac2e65fe7e1804977` / `nightly-2026-10-27`**, with the cross-file None guard. This local build copies the explicitly trusted surviving server source; the prepared release workflow fetches and checksum-verifies the pinned source archive. No toolchain server is overwritten.

The existing `~/.rake` was moved aside, the new version installed through `BLAZE_ARCHIVE`, and the previous version restored alongside it. The actual build had an empty Blaze store and Cargo target, with no Blaze on PATH. Cargo metadata tools remained available through private links. The matching Rust toolchain was reused through a private Rustup cache; this is not a download-time measurement. Compiler/server selection, sandboxing, native hybrid options, `--bin`, 64 MiB stack and automatic `pf-no-simd` remain as documented. The pin's intended future GitHub URL and checksum are updated.

The cold GPUI build finished in **197.21 s**: host build scripts/macros 57.6 s; second workspace load 18.6 s; name resolution 37.2 s; resolved-name storage 3.03 s; std 21.9 s; plus initial load, remaining application/check/link work. There were no macro-expansion failures or refused-function failures. The known malformed upstream `bad-toml` fixture diagnostic still appears without failing the build.

## First edit and six-trial results

The **first edit after the clean cold build** was **1.512 s save → apply / 1.573 s save → inspected redraw**, with the same PID and counter state. It started occluded but became visible; it is reported separately and excluded from the occluded steady-state comparison. [First cold edit](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/pinned-trials/trials.json).

The final measured run reused the warmed store, performed a separately recorded first edit and one warmup of each kind, then six `Live text trial 201..206` edits and six `Helper trial 201..206` edits. The first edit of that warm process was 1.418 s / 1.468 s. All twelve measured edits kept the same PID and **Count: 2**. Before/after native preview records are occluded for every trial. After each save the harness sends only diagnostics/tree queries: no injected input, redraw command or preview frame lease. Redraw is the first completed inspected render containing the marker, not physical scan-out.

| Setup / edit | Save → apply median | Save → redraw median |
|---|---:|---:|
| **Pinned bundle: `view!` text** | **0.959 s** | **1.003 s** |
| **Pinned bundle: helper** | **0.491 s** | **0.594 s** |
| Unchanged lab control: text | 0.991 s | 1.035 s |
| Unchanged lab control: helper | 0.493 s | 0.546 s |
| PR #53 historical text | 0.830 s | 0.897 s |
| PR #53 historical helper | 0.401 s | 0.621 s |

The two-display placement problem was diagnosed using native window bounds. The temporary fixture adds only startup bounds on the editor's display; its render/helper code and markers match PR #53. macOS reported the bundle window as **1510×996** and the lab window as **1536×996** despite the same requested 1536×996 bounds. This small geometry difference limits exact redraw equivalence with the historical reference; compiler phase comparisons use the identical source edits.

Owner-approved background work continued. On **10 logical CPUs**, recorded one-minute load ranged **8.36–14.08** during bundle trials and **19.93–26.83** during lab trials. These are sequential loaded runs, not a controlled idle-machine regression test. [Bundle trials](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/pinned-measured-trials/trials.json), [bundle result](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/pinned-measured-trials/result.json), [lab trials](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/lab-measured-trials/trials.json), [lab result](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/lab-measured-trials/result.json), [phase/load analysis](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/timing-analysis.json), [window placement](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/second-display-window-check.json).

## Where the time goes

All values below are six-trial medians in milliseconds. Source/roots/validation/construction/link are components of request → publication, not additional intervals to add to it. Medians of individual components need not sum to the median total.

| Phase | Bundle text | Bundle helper | Lab text | Lab helper |
|---|---:|---:|---:|---:|
| Save → daemon request | 231.719 | 225.548 | 239.194 | 234.559 |
| Daemon request → publication | 727.141 | 228.031 | 755.236 | 236.366 |
| Source apply / expansion + outlines | 76.128 | 5.396 | 70.030 | 5.087 |
| Roots | 53.911 | 48.279 | 56.984 | 51.014 |
| Cached-code validation | 537.166 | 106.978 | 586.991 | 133.786 |
| Other patch construction | 38.617 | 34.912 | 37.474 | 37.127 |
| Link / publication | 1.087 | 1.252 | 0.883 | 1.038 |
| Publication → applied event | 10.413 | 14.034 | 6.463 | 4.543 |
| Runtime apply (inside delivery interval) | 0.686 | 0.683 | 0.646 | 0.619 |
| Applied event → completed inspected render | 43.573 | 44.286 | 44.163 | 44.807 |

The bundle's text redraw is **106.619 ms** above the PR #53 reference. Its cached-code validation is **85.072 ms** higher (537.166 vs 452.094 ms), source apply about 16.8 ms higher and roots about 9.8 ms higher; delivery/render observation offsets some of that difference. Validation of compiled functions dominates; the patch's runtime application itself is below 1 ms.

**There is no consistent bundle-specific slowdown in this comparison.** The same-edit unchanged lab control is slightly slower for text and essentially equal for helper application. Its validation phase is also above PR #53 (586.992 ms), whereas its native source-apply phase is slightly lower than the bundle's (70.031 vs 76.128 ms). This does not show a server-specific penalty large enough to explain the historical gap; source apply includes outlines and is not isolated proc-macro time. Cargo, rustc and rustdoc are byte-identical, and all 3,724 packaged library files equal the lab files: only their location differs. The evidence points to slower common frontend/cache bookkeeping in these loaded runs, rather than a demonstrated toolchain/library-content or bundled-server regression. CPU contention is a plausible contributor, not a separately isolated causal measurement; a small layout or server effect cannot be bounded from these sequential runs. [Toolchain identity](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/toolchain-comparison.json), [library identity](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/library-comparison.json).

## Regression and integrity

- `cargo test --workspace --locked`: **932 passed, 4 ignored**. Rerun after isolating the fake-tool doctor's HOME.
- Live runtime: **5 passed**; `scripts/test-supervisor.py`: **14 passed**.
- Offline fake-archive installer/checksum tests: **4 passed**. Real installed-bundle doctor readiness also passed.
- Strict CLI Clippy: passed.
- Fork Wasm macro unit suite: **13 passed**, including confined probes and existence-sensitive expansion-cache invalidation; explicit-server path test: **1 passed**; NEON unit tests: **7 passed**.
- Both acceptance Python scripts compile, and an offline guard check confirms existing benchmark evidence cannot be overwritten. `rake format` passed; `rake lint crates/rake-cli` checked 92 Rust files with **0 diagnostics / 0 denied**.

The fake-tool integration fixture now sets HOME to its temporary directory and asserts live mode is not installed/ready there. Previously its doctor checked the real installed archive six times, making an offline fixture depend on user state; the baseline run took 481 s. This is a test-only isolation change, not a production doctor behavior change.

The harness preserves failed setup evidence: removing Cargo with PATH, incomplete occlusion on the wrong display, and the lab runtime's overlong Unix socket path. Those are not included as valid latency samples. The benchmark now checks socket path length before spawning a compiler. Its manual gate permits window verification before measurements.

**All 22 protected binary/archive/manifest hashes are unchanged.** The original Rake checkout's HEAD and existing status are unchanged, and the example source is restored. Both benchmark supervisors/daemons stopped. [Integrity proof](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/integrity-after.json), [regression commands/results](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/regressions.json), [final workspace log](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/workspace-final.log), [Wasm test log](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/fork-wasm-all.log). CI release workflows and public downloads have not been executed. Earlier JPEG differential results were not repeated because reconstructed compiler sources are identical.

## Owner's public steps — not performed

1. Create **`1bye/blaze` as an actual GitHub fork of indexable-inc/blaze**.
2. Push only the reviewed `rake-patches` branch, with its upstream ancestry. Keep `snapshot-archive` local unless deliberately publishing it. Select the patched branch for fork releases.
3. Tag/cut **`v0.20261030.1-rake.1`**. Upload this exact tested archive plus checksums, or run the prepared Apple Silicon workflow and use that build's checksum.
4. Verify the public download. The pin already contains the intended URL and this archive's SHA; change them if the published bytes/repository/version differ. Push/review the Rake branch separately after owner authorization.

A compatible upstream bump is estimated at **2–4 hours**; a Cranelift, rust-analyzer or proc-macro ABI change can require **one or more days**. Review the eight inherited prerequisites as well as the seven requested fixes on each rebase. No first-bump ancestry reconstruction remains necessary.

The local Rake follow-up commit contains this report, pin/docs updates, the comparison harness and test isolation. The exact local commit list is retained in [Rake commits](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-upstream-base-evidence/rake-commits.txt) after committing. Nothing is pushed or posted.
