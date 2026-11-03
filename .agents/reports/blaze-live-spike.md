# Blaze compile and live-edit spike

Measured on 2026-10-28. **Stopped at Gate 1:** none of the five GPUI examples
built under the installed Blaze release. No application or supervisor code changed.

## Environment

| Item | Value |
| --- | --- |
| Branch | `codex/blaze-live-spike` |
| Rake commit | `543ed762c5272b62abf99280bc7ca9a6ab5dabc5` |
| Machine | MacBook Pro, `MacBookPro18,2`, Apple M1 Max, 10 CPU cores, 32 GiB RAM, arm64 |
| OS | macOS 26.6.2, build `25G83` |
| Memory before starting | `memory_pressure`: 44% system-wide memory free; 5,851 raw free 16 KiB pages (91.4 MiB). The percentage includes more than raw free pages. |
| Blaze | `blaze 0.20261027.0 (49ca152)` |
| Blaze commit | `49ca152ec9ae52a31d353e7df9d5a304b0ca097c` |
| Blaze toolchain | `blaze-2026-10-27`, Rust `nightly-2026-10-27` |
| Rake toolchain | Rust 1.95.0; project pin unchanged |
| GPUI CE | `99109e3a77ddadbcb46cba8567e779a651600608` |
| Tiering | Default; neither `BLAZE_TIER` nor its legacy equivalent was changed |

Blaze was initially absent. The owner approved installation in this chat. The
official installer was downloaded, inspected, then run with `--verbose`. It
selected stable release `v0.20261027.0` and downloaded its Rust toolchain (reported
123 MB). Binaries are in `~/.cargo/bin`; Rust is in `~/.blaze/toolchains`.
Plain Cargo was not switched to Blaze. The existing untracked `.agents/plans/`
directory was preserved. Nothing was staged, committed, pushed, or opened upstream.

## Phase 1

Commands were run from the workspace root, in the planned order, as
`blaze build -p <package>`. All exited 101. Each started with no running daemon
and an empty Blaze disk store. Foundation used the initially empty default store;
the other packages used individual empty `/private/tmp/rake-blaze-cold-<name>`
stores via `BLAZE_CACHE`, with `blaze stop` between attempts.

These are first-attempt wall times **to failure**, not completed cold-build
benchmarks. Cargo's existing `target/`, source downloads, and OS caches were
retained. Later attempts may reuse generated build artifacts even though their
Blaze stores were empty. Foundation used `/usr/bin/time -p`; the other attempts
used Python's monotonic clock around the build subprocess, excluding daemon stop.
The Cargo baseline finished before any timed Blaze build began.

| Package | First build attempt | Builds? | Runs correctly? | Edit rebuild | First failing path |
| --- | ---: | --- | --- | --- | --- |
| `rake-foundation` | 148.32 s | No | Not run | Not measured | `gpui_ce_media` build script → `clang_sys::build::dynamic::search_libclang_directories`: `frontend: mir: IncompleteExpr` |
| `rake-view` | 27.510 s | No | Not run | Not measured | Same error |
| `rake-routing` | 24.602 s | No | Not run | Not measured | Same error |
| `rake-notes` | 23.733 s | No | Not run | Not measured | Same error |
| `rake-inspector` | 23.421 s | No | Not run | Not measured | Same error |

First error from foundation, verbatim:

```text
error: could not build `librake_foundation.dylib`, `rake-foundation`: Failed to build the build scripts of <repo>
  blaze could not compile the build script of `gpui_ce_media v0.2.2` for the load
  blaze did not build it (exit status: 1): Error: `build_script_build` reaches 1 function blaze could not compile, and a build with nothing of rustc's has nothing else to compile it, so it is not built:
    clang_sys::build::dynamic::search_libclang_directories: frontend: mir: IncompleteExpr
(what blaze said is in <user-cache>/ra-jit/workers/f950877dd5bc0e0f/log)
```

All five command logs and the foundation worker's diagnostic are preserved in
[blaze-live-spike-builds.txt](blaze-live-spike-builds.txt).

`cargo tree --locked -p rake-foundation -i clang-sys` confirmed `clang-sys 1.9.1`
through `bindgen 0.71.1`, a build dependency of GPUI CE and `gpui_ce_media 0.2.2`.
The pinned media build script invokes bindgen on its Objective-C header.
The diagnostic is a Blaze frontend/MIR compilation failure, before a usable GPUI
executable exists; it is not a reported missing-libclang error or a signing failure.
**Inference:** Blaze needs to compile this build-script dependency path before
Rake can be evaluated further. No GPUI CE, clang-sys, or Rake workaround was tried.
Later compilation and runtime compatibility remain unknown.

### Cargo baseline

`cargo test --workspace --locked` passed on Rust 1.95.0, exit 0: 298 passed,
0 failed, 0 ignored across 44 test-result groups, including doctests. The UI
compile-fail suite reported 75 passing fixture cases within its test harness.

The initial sandboxed invocation failed seven HTTP tests because localhost socket
binding returned `Operation not permitted`. Repeating the same full command with
the required host permissions passed, including those HTTP tests. This was an
execution-environment restriction, not a source change. Full baseline logs remain
at `/private/tmp/rake-blaze-cargo-test-unrestricted.log` and the initial run at
`/private/tmp/rake-blaze-cargo-test.log`.

## Phase 2

Not started because Gate 1 failed. No harness or live dependency was added.

| Edit kind | Outcome | Reason | Save-to-pixels |
| --- | --- | --- | --- |
| Label/function body | Not tested | No example built under Blaze | Not measured |
| Color | Not tested | Gate 1 failed | Not measured |
| `view!` class | Not tested | Gate 1 failed | Not measured |
| Add a view field | Not tested | Gate 1 failed | Not measured |

No window, repaint, signing, or `Applied`/`Restart`/`Declined` behavior was tested.
No screenshot was taken because no Blaze executable was available to launch.

## Phases 3–4

Not started. The reload-boundary design choice remains open; no decision about
`GenerationId`, cancellation, or route detachment was made. No feature,
dependency, compiler flag, doctor output, or supervisor behavior was changed.

## Sources inspected

Documentation was read at the installed commit rather than moving `main`:
[README](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/README.md),
[usage](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/docs/usage.md),
[live integration, statics, refusals, and signing](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/docs/live.md),
[installation](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/docs/install.md),
[Cargo integration](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/docs/cargo.md),
[live API](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/crates/ra-jit-live/src/lib.rs),
and [windowed demo](https://github.com/indexable-inc/blaze/blob/49ca152ec9ae52a31d353e7df9d5a304b0ca097c/demo/life/src/window.rs).
This release retains the old `ra-jit-live` source/package name; the plan's
`crates/blaze-live/src/lib.rs` path does not exist at this commit. No live
dependency version was selected because Phase 2 was not reached.

## Recommendation

**Wait for Blaze to mature on this dependency chain.** Release 0.20261027.0 cannot
compile the bindgen/clang-sys code required by GPUI CE's media build script, so
neither a native application nor live editing can be evaluated on this Rake
revision. Retry Phase 1 with a Blaze release that handles the recorded
`IncompleteExpr` failure; proceed to the throwaway live harness only after a GPUI
example builds and runs. The passing Cargo baseline supports keeping the existing
compiler and restart flow. These measurements do not justify adopting or
permanently rejecting Blaze, and say nothing about later Blaze revisions.
