# GPUI CE / Blaze: native app and live reload

2026-11-03. The native GPUI app now builds and runs with the locally patched Blaze compiler. The window renders text and styled content, counts clicks, receives keyboard input, resizes, and exits cleanly through Escape and the close button. Live tests now demonstrate body edits, state preservation, compile-error recovery and layout-change restart detection. Practical repaint and worker-recovery limits remain; see the live-test results below.

The isolated lab is `/Users/yuriihulyk/Documents/GitHub/blaze-gpui-lab`. GPUI and accesskit source remain unchanged, including the CGRect accessibility methods. The only dependency workaround is the plan’s **test-only scalar pathfinder feature**, `pathfinder_simd = { version = "=0.5.6", features = ["pf-no-simd"] }`. It trades SIMD performance for compiler compatibility; it is not an implementation of the missing SIMD intrinsics. No JPEG workaround or native-proc-macro exception was needed. Rake changes are limited to its report. Nothing was pushed or posted upstream.

## Tested revisions

| Item | Value |
| --- | --- |
| Machine | MacBookPro18,2, M1 Max, 10 cores, arm64, 32 GiB |
| OS | macOS 26.6.2 / 25G83 |
| Stable Blaze, preserved | 0.20261027.0 / 49ca152ec9ae52a31d353e7df9d5a304b0ca097c |
| Official canary / source base | 0.20261030.1 / 6560553cc09f00d8d96edc24d85a5e441352ab7c |
| Local Mach-O fix | 7565ed96bb1163f5d55c0ab867b2dfa56bd89921 |
| Local C-ABI fix | bdff49602c724191718356c85887791032db24e2 |
| Tested engine SHA-256 | ea0baa8264e3f02a4bdfd09cf9e71693b6c862560249ef638a95bc4428000f09 |
| Pinned Rust | nightly-2026-10-27 / 1.101.0-nightly / 16d1ffac17ad8b739e32483ac2e65fe7e1804977 |
| GPUI CE | bc577476a0eb55173ef985585cfb487f75ab1cab; gpui-ce 0.2.2 / gpui_ce_platform 0.1.0 |
| Current lockfile SHA-256 | d8138db071d9c757bb37efe0b6a113fb3875781026fc17eacde6077b641e09fa |
| Original lockfile SHA-256 | 2f2afb32da3273c470d8e116d3ac4abbae37864eacfe468b0501a71919cf7495 |

The current lockfile adds the scalar root dependency and the local, pinned blaze-live 0.1.0 runtime package/root entry. Released dependency versions did not change. Default GPUI features and platform font-kit remain enabled. The original stable installation and isolated official canary remain intact. The tested engine is in `tools/abi-fixed-blaze-bin`; the older linker-only engine remains in `tools/fixed-blaze-bin`. Both local commits are on `codex/macho-link-name-20261103` in `/Users/yuriihulyk/Documents/Codex/2026-11-03/pleas/work/blaze-source/blaze-macho-link-fix-20261103`.

The engine was self-built using the official canary with `blaze build --release -p blaze-engine --bin blaze-engine --locked`, in 535.411 s. No Cargo/rustc fallback was added to Blaze. Its embedded version names parent 7565ed9 because it was built before the ABI commit; the recorded binary hash identifies the tested code. CLI/rustc wrappers and the official proc-macro helper were preserved from the preceding installation. The local engine lacks LLVM support, so application runs use `BLAZE_BACKEND=cranelift`.

## What removed the blockers

The released canary first failed linking GPUI’s shader build script. Its object writer treated the byte-1 exact-symbol marker in `link-section` names as a literal character and added the Mach-O underscore. The local fix handles exact named functions/data and forwards static `#[used]` retention while preserving explicit compiler-only retention. The unchanged reduced program now reads 42 instead of failing to link or reading a stripped section. The original full shader script subsequently linked and ran.

A full linker-only GPUI build then stopped on five pathfinder SIMD refusals and one accesskit CGRect projection. Adding only `pf-no-simd` removed all five SIMD refusals; the 485.239 s rebuild still failed on CGRect. This isolates the feature workaround’s effect.

The remaining compiler bug was that layout normalized an associated type, but aggregate C-ABI field classification still inspected its unresolved projection. The fix normalizes the type before walking aggregate fields in `scalar_leaves` and `classify`. It uses the existing normalization helper, without special-casing GPUI, CGRect, objc2 or crate names. A permanent breaker covers old `__Inner`, new `Inner`, projected arguments/returns and a nested homogeneous float aggregate. Unresolvable projections are still refused.

The diagnosis corresponds to [Blaze #1259](https://github.com/indexable-inc/blaze/issues/1259), and the commit credits the issue author and the comment identifying both projection forms. Main was checked at 0ca1bcd546d4f75a9e0e4a98dca3b862bfc03882 and still lacked this normalization; the patch stays on the canary base. The SIMD limitation remains associated with [#1260](https://github.com/indexable-inc/blaze/issues/1260).

## Build and compiler validation

| Check | Result |
| --- | --- |
| Cargo build with scalar feature | Pass, 39.062 s; warm/resumed dependencies |
| Scalar feature + linker-only Blaze | Fails only on CGRect, exit 101 |
| ABI-fixed full GPUI build | **Pass, exit 0, 581.440 s**; all 139 preparation steps; 116,438 functions in 461 crates |
| `blaze run --locked` | Builds its runnable artifact in reported 392.75 s, then starts native PID 70474 |
| Permanent breaker, unchanged source | Old compiler refuses; new compiler passes; pinned Cargo passes |
| Clang/Rust ABI oracle, both call directions | Old compiler refuses; new compiler and pinned Cargo pass; NaN payload, negative zero, infinity, subnormal and nested aggregate checks |
| Actual objc2 0.5 CGRect callback | Fixed Blaze and pinned Cargo pass |
| Actual objc2 0.6 CGRect callback | Old compiler refuses; fixed Blaze and pinned Cargo pass |
| Updated `hir-ty-codegen` unit suite | 53 passed, 0 failed, compiled/tested with official Blaze; 183.991 s including build |
| Earlier linker validation | 538 hir-def tests passed, 1 ignored; 53 codegen tests passed; exact-symbol object tests and used-retention comparison passed |
| Source formatting and whitespace | Modified Rust files pass rustfmt; source commit passes `git show --check`; lab code and shell syntax pass |

Standalone fixture sources and lockfiles are retained in `repros/`, with commands in its README. The permanent breaker’s exact source was exercised as a standalone program; the complete breaker harness was not run. The full contribution gate, new-engine LLVM backend, Linux/Windows execution and release train were not run. Local Darwin execution was explicitly requested by the owner despite the source repository’s ix-VM-only guideline; ix and the pinned LLVM installation were unavailable. These are local validation results, not an upstream merge-readiness claim.

All current builds disable the swap guard under the owner’s explicit instruction. `RUST_MIN_STACK=67108864` is a test-only 64 MiB compiler-thread stack setting required to get past the earlier Wasmtime/Cranelift stack overflow. That overflow was a stack-guard crash, not an OOM diagnosis. Resource samples include concurrent compiler probes, so their global process peaks cannot be attributed solely to GPUI. Timings reuse caches and are not cold-build benchmarks. At the earlier native-build resource check macOS reported 81% system-wide memory free and about 5.66 GiB swap still allocated; allocated swap alone is not proof of active pressure. The earlier build-test daemons were stopped. The live tests leave the verified native app and its warm isolated daemon running for the owner.

## Runtime comparison and one observed limitation

The same app source and scalar feature were built by both compilers. Cargo’s root crate was explicitly cleaned and rebuilt in 4.102 measured seconds before preserving the reference, to avoid mistaking Blaze’s shared output path for a fresh Cargo result. The original Cargo reference is also preserved separately.

The UI tool requires a registered macOS app bundle. Minimal test-only bundles contain byte-identical copies of the compiler outputs; no renderer or source changes were used for packaging. Source/copy hashes are saved in `evidence/BlazeGPUI-runtime-bundle.json` and `evidence/CargoScalarReference-runtime-bundle.json`:

- Blaze runnable SHA-256: `6a7f37b37f0238cbae499742ad4d7e514e541d666e5402dd2786723e9be9be39`.
- Matching Cargo SHA-256: `9619df4f27b307432588fb0710c8b54c2a57c45a0670dc8a9f17fffb4000e5f6`.

Both display the same centered text, font, colors and counter using the default macOS/Metal path. Cargo recorded click 1 and key `a` 1, then Escape exited 0. Blaze recorded clicks and `a`/`b` events, Escape exited 0, and a separate run resized by native window zoom/unzoom and closed through the close button with exit 0. The saved Blaze screenshot shows clicks 3 and keys 4. No panic, Objective-C exception or crash appears in the captured native-run logs.

**A repaint difference remains:** in the Blaze run, key events update the logged state immediately, but the displayed key count can stay stale until another mouse interaction or resize. Cargo showed its key count after the single key event. Resizing revealed the correct Blaze count, and Escape quits immediately. This is an observed runtime limitation, not a proven root cause; it has not been reduced or fixed. No app-level refresh workaround was added to hide it. A successful build and native window therefore do not establish full behavioral parity.

`blaze run`’s unbundled process was deliberately terminated with SIGTERM after startup because the UI tool could not address an unbundled app. Its recorded exit 143 is that controlled cleanup, not a compiler or app crash. The byte-identical bundled GUI runs independently establish the clean exit-0 results. The verified live-enabled window is now left open as PID 85318 after the subsequent live tests.

PNG/SVG rendering, separate GPUI examples and actual VoiceOver operation remain untested. Accessibility code was preserved, but preserving it is not proof of VoiceOver correctness. The live matrix was subsequently exercised: text, styling and helpers each have ten rendered trials, with two additional styling timeout samples retained. Basic patching is demonstrated; missed repaint deadlines and worker/bundle recovery prevent an unqualified unattended-reliability claim for Gate 2. This is not a recommendation to move Rake to Blaze yet. Rake’s older GPUI pin was not attempted. No remote devbox installation/build was performed.

## Live reload results

The pinned `blaze-live` runtime is polled every 20 ms on the GPUI UI thread, with a window refresh on `Applied`. The run recorded 43 applied events. Ten rendered trials each for text, styling and helpers preserved process/state; median save-to-render times were 18.140 s, 17.926 s and 17.895 s. Styling required two extra trials after deferred-frame timeouts. New helpers, changed click closures, background calls, compile errors/fixes and ten rapid saves also worked. A view-field edit returned `Restart` naming `Counter`; an owned static initializer applied instead of restarting.

A worker recycle rejected the first post-restart socket request while loading. Its subsequent full relink did not notify the separate copied bundle, requiring a manual bundle sync/restart. Two body edits then applied in PID 85318, preserving counter 2. Final source matches a pinned Cargo reference. The full details, timing qualifications, failures and handoff are in [the live-reload report](/Users/yuriihulyk/Documents/Codex/2026-11-03/pleas/outputs/gpui-live-reload-results.md). The live results are saved locally; no compiler changes or upstream posts were made during this phase.

## Reproduce

From the lab:

```sh
BLAZE_TIER=off BLAZE_LAB_SWAP_GUARD=0 ./run.sh fixed-build
BLAZE_TIER=off BLAZE_LAB_SWAP_GUARD=0 ./run.sh fixed-run
```

The helper chooses the ABI-fixed compiler, isolated home/cache, Cranelift backend and 64 MiB thread stack. `./run.sh fixed-stop` stops that isolated compiler daemon. `BLAZE_LAB_RUN_NAME` selects a new build/run evidence name; archive existing names before reusing them. The monitor samples resources but the commands above deliberately disable swap-based termination. Cargo commands remain `./run.sh cargo-build` / `cargo-run`; they use the pinned nightly.

For the already built, UI-addressable native apps, open `tools/BlazeGPUI.app` or `tools/CargoScalarReference.app`. The original non-scalar Cargo reference remains `tools/CargoReference.app`. The bundles, binaries, target directories and caches are ignored by git; sources, patches and evidence are retained.

The prior full report is archived as `evidence/report-before-abi-fix.md`, retaining the original refusal list, stack crash and resource history. Current logs are `gpui-phase1-abi1*`, `gpui-phase2-run*`, `gpui-phase2-blaze-bundle*`, `gpui-phase2-blaze-final*` and `gpui-phase2-cargo-bundle*` in `evidence/`. The compiler ABI patch is saved alongside the earlier Mach-O patch. No upstream changes were published.

## Profiling follow-up — 2026-11-03

The native profile identified WebAssembly procedural macro replay as the main bookkeeping cost (78.2% of sampled main-thread stacks under the WASM expander). The lab's unique `BLAZE_LAB_RUN_NAME` was leaking into the compiler environment and triggering a replay before the source edit triggered another. `measure.py` now consumes that label only in the harness. The compiler binary is unchanged.

In the CPU-accounted confirmation, three warm trials per case improved median save-to-apply **18.890 → 9.634 s**, CPU-side render **18.891 → 9.638 s**, and worker CPU **18.66 → 9.95 s**. All 20 timed patches rendered with preserved state in PID **54153**. Adverse shared-load pilot results, setup rebuilds, slow error recovery and first-request environment-transition costs are retained; these are not guaranteed latency numbers. The remaining roughly 9 seconds is primarily the required source-revision macro/name-resolution replay.

See `LIVE_PERF_REPORT.md` in the lab and `evidence/live-perf/summary.json` for the full profile, before/after measurements, exclusions and verification. The current native app is PID **54153**, counter **1**, keys **2**, restored to the original blue source; `evidence/live-owner-handoff.json` and ignored `tools/live-owner/runtime.log` identify the active session. No Rake application code or dependency pin changed.
