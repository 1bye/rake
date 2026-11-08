# Pinned Rake Blaze — local preparation

The final archive installs from `BLAZE_ARCHIVE` into a clean `~/.rake`, with no Blaze on `PATH`. `rake dev --live` builds and runs the GPUI example, and both a `view!` text edit and an ordinary helper edit apply live with the same PID and counter state. All public steps remain unperformed.

## Local repositories and provenance

- Fork: `/Users/yuriihulyk/Documents/GitHub/blaze-rake-fork`, branch `rake-patches`.
- Rake: `/Users/yuriihulyk/Documents/GitHub/rake-worktrees/blaze-pinned-live`, branch `codex/blaze-pinned-live-20261108`, from PR #53's `ee0ec974b82b08a9cab83dd7f3d96cc8e3a70476`.
- The original `~/Documents/GitHub/rake`, the previous engine source and its binaries/toolchain are not modified.

**Provenance limit:** `2007d7c8c849fd5a7858d5816e08764d9042c87c` is the recovered lab's root snapshot, not a verifiable upstream commit. The PR #53 build records identify exactly that source and the retained lab diff. Original upstream ancestry is unavailable. WASI realpath and vendored `span_join` were already present in the snapshot; their separate commits preserve them and add regression/packaging support. The remaining five commits import the tested lab changes. This must not be described as seven clean backports on a known upstream Git ancestor.

Seven patch commits plus release preparation:

- `0f6141f` fix(wasi): preserve confined realpath probes
- `3471b38` fix(macros): select the patched native span server
- `e7bfce0` fix(codegen): give std its own symbol namespace
- `a49e6fc` fix(attrs): resolve module paths in symbol names
- `7b3fa17` fix(toolchain): give macros canonical Cargo paths
- `284b90d` fix(probes): use real rustc in hybrid host mode
- `fd7924b` fix(neon): lower the GPUI dependency intrinsics
- `6a209f7` build: prepare pinned Apple Silicon fork releases
- `0641b76` build: link stock-toolchain macro servers

The std fix covers fresh/cached naming and changes its cache key **5 → 6**. Symbol attributes share the owning-module substitution. Native hybrid build-script probes keep sandboxing. The new explicit native-server path resolves aliases and rejects missing/non-file paths without panicking. The codegen, NEON, symbol attributes, host-probe and Cargo-path files match the tested lab bytes; [parity record](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/lab-patch-parity.json).

## Release archive

- Version: **`0.20261028.0-rake.1`**.
- Built source: **`6a209f79cf1a64d9f684bd59572961a9411c2f45`**.
- Target: **`aarch64-apple-darwin`** only.
- [Archive](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/release/blaze-v0.20261028.0-rake.1-aarch64-apple-darwin.tar.gz).
- SHA-256: **`a7675a36d6c60b1787ea1e2ca13007ec51d7276875b080a7107d3c9b421c9801`**.
- [Checksum file](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/release/SHA256SUMS), [build configuration](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/build.json), [final build log](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/build-verified.log).

The fork head also includes build-only follow-up `0641b76`, which supplies the matching native-library search path when CI installs rustc-dev directly in the sysroot. The tested archive was built from `6a209f7`; compiled engine/server sources are unchanged by that follow-up. Its external-rustc-dev local build already used those link flags. Future workflow builds must use their own checksum.

The normal `blaze-v<V>-<target>/` archive root contains `blaze`, `blaze-engine`, `blaze-rustc`, licenses/README/changelog, the separate fixed `rust-analyzer-proc-macro-srv`, its matching `server-lib` libraries, `RAKE_BLAZE.json` and `share/blaze/library`. It uses the previously authorized pinned Cargo release bootstrap, matching the lab's optimized host compiler build, without LLVM tiers/PGO. It does not claim a self-hosted upstream distribution.

The macro server comes from Rust **`16d1ffac17ad8b739e32483ac2e65fe7e1804977`**, matching `nightly-2026-10-27` and the toolchain pins. The build script checks those identities, applies the same cross-file guard to the native server source and signs the staged binaries after replacing build-machine rpaths with loader-relative ones. No toolchain server is overwritten. `harness/toolchain_bump.py` updates the upstream toolchain/vendor inputs; the fork additionally requires updating `rake/toolchain-server.json`, and refuses a mismatch. A developer-supplied local server source directory is an explicit trusted build input; the release workflow uses the pinned downloaded source archive and SHA.

The upstream installer copies only its known binaries and does not install this additional native-server/library layout. Rake therefore uses its own authenticated archive installer. All upstream workflows are archived under `.github/upstream-workflows`; only explicit fork `release.yaml` and `release-cut.yaml` remain active. They target Apple Silicon, use fork tags `v0.YYYYMMDD.N-rake.N`, and do not promote channels, publish crates or invoke Indexable infrastructure. These workflows are **prepared but not run**. GitHub's [runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) identifies the selected `macos-15` standard runner as arm64.

## Rake behavior

The pin is `crates/rake-cli/blaze-pin.toml`: `1bye/blaze` and its future release URL are placeholders; the SHA above already verifies the local bytes. Only `--live` selects this installation. Plain `rake dev` remains Cargo-based; legacy `--compiler blaze` remains the separate laboratory path.

- Missing compiler: download HTTPS bytes or use `BLAZE_ARCHIVE`, check the pinned SHA before extraction, reject traversal/links/special files/duplicate files, bound archive size, and atomically install in `~/.rake/blaze/<version>/`.
- Later launches: verify the saved archive and compare the installed manifest and every recorded file with it. Wrong checksums, damaged files and non-executable compiler/server files fail; no PATH fallback or silent replacement occurs.
- `RAKE_BLAZE` is an explicit developer override requiring the patched server and companion engine/wrapper beside it; it bypasses the archive pin.
- Live builds enter the exact `blaze-engine cargo build` entry directly, so the thin launcher's workspace version redirect cannot select another release. Build errors still use the structured Blaze decoder.
- Both build/app environment and direct daemon edits use the version-specific private runtime directory. The explicit fixed-server path, hybrid flags, sandboxing, `LIBLZMA_NO_PKG_CONFIG=1`, 64 MiB stack and `--bin` are set by Rake. Existing memory limits are retained.
- The app's existing Rake `live` feature automatically enables `pathfinder_simd/pf-no-simd` on macOS; no additional app manifest edit is needed.
- Unsupported platforms receive an Apple Silicon macOS error before installation. Doctor reports the pin/platform/archive/file checks and server presence without downloading; missing optional live mode does not fail an otherwise-ready Cargo doctor.

[Development instructions](../../docs/development.md).

## Real acceptance

The final run starts with `~/.rake` absent and no `blaze` executable on the child's PATH. It installs the exact archive above, builds the binary with the automatic live feature, displays the test-only text/helper and seeds the counter to **1**. One text edit and one helper edit produce `live_patch_applied` and completed inspected renders. Neither restarts the process; counter **1** remains. No preview input/redraw is sent during timing.

| One trial immediately after the cold build | Save → app apply | Save → observed redraw |
|---|---:|---:|
| `view!` text | **1.964 s** | **2.042 s** |
| Helper | **0.717 s** | **0.766 s** |

These are individual first-edit observations, not warm medians or a replacement for PR #53's six-trial table. Redraw is the completed inspected render, not physical scan-out. Both trials keep PID **44726**. App patch application itself takes 852 µs / 614 µs.

Final cold compiler build: **173.93 s**; complete install/metadata/build startup: **213.50 s**. The exact Rust toolchain was cached outside the clean Rake home for this final run. After acceptance, that same verified toolchain was copied into the final version-specific runtime cache so future normal launches do not depend on the isolated test Rustup home. Normal-environment doctor also reports ready. An earlier complete clean run also proved automatic toolchain download (117 MB), then built in 186.16 s and applied both edits. Its measurements are preserved separately, since a compiler regression build was running then and it used the preceding test-only source revision.

[Final result](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/real-live/result.json), [reload records](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/real-live/reloads.json), [doctor readiness](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/real-live/doctor.json), [supervisor log](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/real-live/supervisor.jsonl), [build output](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/real-live/supervisor.stderr), [acceptance configuration](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/acceptance.json). `scripts/test-blaze-live.py` is opt-in; it restores fixture source and stops only its owned app/daemon. No fixture changes are committed.

The first packaging attempt excluded every directory named `target`, accidentally removing `library/vendor/cc/.../src/target/apple.rs`. The clean app build caught it before compilation. The recipe now copies all library sources. [Preserved failure](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/acceptance-library-failure.log). A path-test assertion initially used an unavailable wrapper method; it was corrected to convert to `PathBuf`, and both fork regressions pass before the final archive build.

This run covers installation and two body patches. It does not repeat PR #53's complete edit/restart/64-edit matrix; those known experimental limitations remain.

## Verification and remaining public steps

- **932 workspace tests passed, 4 ignored** (`cargo test --workspace --locked`), including all 7 new CLI tests.
- **5 live-runtime tests passed**, with `live,view-test-support`.
- **14 supervisor tests passed**.
- **4 offline installer tests passed**: installation/file tampering, wrong archive SHA, authenticated manifest tampering, and path confinement. These run without network or Blaze.
- **2 fork regressions passed**: WASI prefix/existence/read-set checks and explicit native-server path handling. Both use the pinned compiler; the WASI test's six-minute duration is compilation, with the selected test itself completing immediately.
- Strict CLI Clippy (`--all-targets -- -D warnings`) passes. Rake formatting/linting is checked before the local commits.
- The real archive acceptance and doctor checks above pass. Default Cargo lifecycle behavior is covered by the unchanged supervisor fixtures and workspace regression; no new plain-Cargo GUI benchmark is claimed.

[Verification summary](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/verification-summary.json), [workspace output](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/workspace-tests.log), [fork unit results](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-fork-evidence/fork-unit-results.json).

Owner-only public steps are next; none is executed here:

1. Create `1bye/blaze` (the recovered history is a downstream snapshot, not a proven upstream fork ancestry).
2. Review and push the local fork branch, including licenses, seven patch commits and release recipe. Choose its default branch explicitly; do not enable the archived upstream infrastructure workflows.
3. Cut `v0.20261028.0-rake.1`. Upload the exact tested local archive to preserve its SHA, or run the prepared release workflow and use that build's actual checksum.
4. Confirm/fill the pin's repository, public archive URL and SHA against the published bytes. A CI rebuild can differ; do not copy the local SHA blindly. Rerun offline/real archive acceptance if bytes change.
5. Separately authorize/review any push of the new Rake branch. This task performs no push, PR update, repository creation, release or upstream post.

For compatible upstream bumps, budget roughly **2–4 hours** to transplant patches, check same-revision server pins, build and run acceptance/differential checks. A Rust/proc-macro/Cranelift ABI change can take **one or more days**. The first bump has extra unbounded provenance work: recover the true upstream base and account for fixes already inside the recovered root. `RAKE_FORK.md` documents the manual upgrade sequence and the opt-in acceptance launcher; no automatic rebase onto an invented upstream ancestor is claimed.
