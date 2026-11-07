# Plan: get a plain GPUI app working under Blaze

Handoff plan. The goal is one small GPUI CE app on macOS (Apple silicon) that
builds with Blaze, behaves the same as its cargo build, and takes live edits
through `blaze-live`.

**This is not Rake work.** Don't change anything in the Rake repository except
adding the report at the end. Rake is only here as a later consumer.

Work in phases and stop at each gate. Phases 0-2 change no Blaze code. Prefer the
smallest thing that proves the next step, and label every workaround as test-only.

## Why this might be close

The first spike (`.agents/reports/blaze-live-spike.md`) stopped because Blaze could
not compile a `clang-sys` function in a GPUI build script. That issue,
[#1349](https://github.com/indexable-inc/blaze/issues/1349), is now closed. One
doubt: a comment reports the same error on `clang-sys` 1.9.1 on canary
`v0.20261029.0`, after the fix PRs merged. It was closed about five hours later
with no stated reason, so treat it as unconfirmed.

Another team reported the next failures in their GPUI app in
[#1303](https://github.com/indexable-inc/blaze/issues/1303). All were still open
on 2026-11-03:

| Issue | Refused | Crate | Likely way around it |
| --- | --- | --- | --- |
| [#1260](https://github.com/indexable-inc/blaze/issues/1260) | `floor.v2f32`, `ceil.v2f32`, `round.v4f32`, `simd_minimum/maximum_number_nsz` | `pathfinder_simd` 0.5.6 | Its `pf-no-simd` feature selects the scalar code |
| [#1260](https://github.com/indexable-inc/blaze/issues/1260) | `aarch64.neon.sqshrun.v4i16` | `zune-jpeg` | Its default `neon` feature can be turned off, or JPEG avoided |
| [#1259](https://github.com/indexable-inc/blaze/issues/1259) | `CGRect` projection in the C ABI (`__Inner` on objc2 0.5, `Inner` on 0.6) | `accesskit_macos` 0.26.x | No clean switch; see Phase 1 |
| [#1258](https://github.com/indexable-inc/blaze/issues/1258) | `sha2` too many Wasm locals | `rust-embed` | GPUI's `embedded-assets` is off by default; or `native-proc-macros` |

These are from another app and may not be the full list for this one. Passing the
build also proves nothing about behavior: this is a different compiler, and GPUI is
heavy on Objective-C, Metal and C ABI calls. #1303 also reports a 22 GB worker on a
~485-crate app, and this machine had 32 GiB.

## Settled decisions (don't relitigate)

- **Outside Rake.** Work in a new directory, such as
  `~/Documents/GitHub/blaze-gpui-lab`, with its own git repository. Tell the owner
  the path.
- **A plain GPUI CE app.** No Rake crates.
- **Pin one GPUI CE revision first:** `bc577476a0` (main when this was written).
  Try Rake's current pin, `99109e3`, only after success, and only as a build check.
- **Cargo stays the reference.** Each behavior check compares against a cargo build of
  the same source on Blaze's pinned nightly. Keep a cargo baseline working throughout.
- **Don't patch GPUI CE** unless Phase 1 leaves no other way. Dependency `[patch]`
  entries go in this lab repository only.
- **Don't open issues or PRs, or comment upstream, without asking the owner.**
- Keep the existing stable Blaze working if you can. Read `install.sh` first and
  install the canary somewhere isolated if it supports that; otherwise tell the owner
  it will replace `0.20261027.0` and how to go back with `--version`.

## Before you start

- Ask the owner before installing Blaze releases. Use the newest release at that time.
  On 2026-11-03: stable was `v0.20261027.0`, the one that failed; the newest canary
  was `v0.20261030.1`, which [failed its promotion gate](https://github.com/indexable-inc/blaze/issues/1570).
  Canary needs `--channel canary` or `--version`.
- Don't start from Blaze `main`. It carries in-progress "v2" checkpoints, and has moved
  past the last release. Only Phase 3 builds from source.
- Read in the Blaze repository at your installed tag: `docs/live.md`, `docs/usage.md`,
  `docs/cargo.md` (for `native-proc-macros`), `docs/support.md` and
  `crates/blaze-live/src/lib.rs`. The first spike found the live crate under its old
  name at an older tag (`ra-jit-live`), so check the name at your tag.
- Record the machine, free memory, Blaze version and commit, GPUI CE revision and
  `Cargo.lock`, and watch memory during each build. Stop the daemon and report before
  the machine starts swapping heavily.
- `BLAZE_CACHE=<dir>` and `blaze stop` give each timed attempt a clean start, as in the
  first spike.

## Phase 0: Get the real failure list (no workarounds)

1. Create the app: one window, text, a styled `div`, a click handler that changes a
   counter, using `gpui-ce` and `gpui_platform` at the pinned revision. Check
   `crates/gpui/examples/` in GPUI CE for current startup code.
2. Confirm it builds and runs with cargo on Blaze's pinned nightly. If it doesn't, fix
   that first. Save a screenshot as the reference.
3. Run `blaze build`. Save the **complete** refused-function list and the
   `blaze`-reported worker log.
4. If the old `clang-sys` error is back, stop. Record the versions, then check
   `bindgen` and `clang-sys` versions in the lockfile against #1349. Report to the owner.
5. Sort each refused function into: known issue, or new. For each, use `cargo tree -i`
   to show why it's reachable. Don't assume it matches #1303.

**Gate 0:** report the list before moving on if there are more than about 15 refused
functions, or anything fundamental (the compiler can't get through GPUI's own code
rather than a few leaf functions).

## Phase 1: Remove blockers with test-only workarounds

Change one thing at a time and rebuild, recording what each removes. Use the
least invasive option that works:

1. **`pathfinder_simd`:** add it as a direct dependency at the locked version with
   `features = ["pf-no-simd"]`, so Cargo's feature unification selects scalar code.
   Check that `cargo tree -e features` shows it, and that the refused functions go away.
2. **`zune-jpeg`:** `cargo tree -i zune-jpeg -e features` first. GPUI's newer
   `image-default-formats` feature can be disabled, but `resvg`'s `raster-images` may
   still pull it in. If it's still reachable, use a local `[patch.crates-io]` copy with
   `neon` removed. Record that JPEG now uses scalar code.
3. **`accesskit_macos`:** use a local `[patch.crates-io]` copy at the locked version,
   and remove or change only the methods that return `NSRect` (`frame` at about line
   452 and `frame_for_range` at about line 936 of `src/node.rs` in 0.26.3). Expect
   VoiceOver and accessibility to be degraded. This is the one workaround that loses
   behavior; say so in the report. If removal breaks required protocols or crashes,
   skip to Phase 3 for this one.
4. **Anything new:** try a feature flag, then `native-proc-macros` for proc macros, then a
   small `[patch]`. Patching GPUI CE itself needs an explicit note on why nothing else worked.

Keep the patches small and readable: one directory per crate, with a README line
giving the original version and the change.

**Gate 1:** `blaze build` succeeds. If not, report the remaining list and go to Phase 3
only if the fixes look small.

## Phase 2: Does it behave, and does it take live edits?

**Behavior**

Run with `blaze run`, and compare against the cargo build with the same patches, so that
the workarounds aren't mistaken for Blaze differences. Check:

- the window opens with correct content, with a screenshot compared to the reference
- text and fonts, and the Metal renderer
- click, keyboard input, resize, close and quit
- one PNG or SVG image if the app can show one simply
- logs for crashes or Objective-C errors

Then build up by running a few of GPUI CE's own examples the same way, such as
`hello_world`, `text`, `animation` and `image_loading`, as a separate Cargo app if they
can't be run from inside the GPUI CE checkout. A difference from cargo is a key finding:
reduce it as far as you reasonably can.

**Live edits**

1. Add `blaze-live`. Call `Live::start()` at startup and log the result. Poll it on the
   UI thread from a foreground timer every 16-30 ms. Don't use `apply_as_they_come`.
2. On `Applied`, call `cx.refresh_windows()` and log the outcome. Without the refresh,
   GPUI may reuse cached output, so an edit can look like it failed.
3. Run this table. Record the outcome, the reason string, save-to-visible time, whether
   the PID stayed the same, and whether the counter value survived:

| # | Edit | Expected |
| --- | --- | --- |
| 1 | Text in `render` | `Applied` |
| 2 | A color or padding value | `Applied` |
| 3 | A helper called from `render` | `Applied` |
| 4 | A new helper, then call it | `Applied` |
| 5 | A click handler closure body | `Applied`; check the old closure isn't kept |
| 6 | A background task body | Next call runs new code |
| 7 | Add a field to the view struct | `Restart` naming the type |
| 8 | A static's initial value | `Restart` |
| 9 | A compile error, then its fix | App keeps running, and the fix patches |
| 10 | Ten quick saves | Last one wins |

Run cases 1-3 at least ten times each for medians. For comparison, record a cold build and
a one-line edit rebuild with cargo, and peak daemon and worker memory.

`docs/live.md` warns that the program may not always be told about a relink, and that the
copy in `target/debug` differs from the daemon's. Note if a restart is needed but not
reported, and test that patches still arrive after a restart.

**Gate 2:** a working app that takes edits 1-3 reliably is the success condition. Stop and
report to the owner, even if everything passed, before starting Phase 3.

## Phase 3: Fix Blaze itself (only with owner approval)

Do this if Phase 2 succeeded and the owner wants the workarounds gone, or if a blocker
has no workaround. Time-box it to **one day**.

1. Check the issues and `main` again first; a fix may have landed, possibly in the v2 work.
2. Follow `docs/contributing.md`: build from source, and run the gate it requires on
   macOS. Say what you ran and what you didn't.
3. Start with the likely smallest pieces, without assuming they are small:
   - `aarch64.neon.sqshrun.v4i16`: `crates/hir-ty-codegen/neon.rs` holds the NEON table.
   - The `floor`, `ceil` and `round` vector intrinsics and `simd_{minimum,maximum}_number_nsz`.
   - #1259: normalize the associated-type projection before classifying the C ABI, for
     both objc2 versions. The repros are in #1259 and #1303.
4. For each fix, add the tests Blaze expects and check results against rustc. A SIMD fix
   needs exact edge cases, including NaN handling, rounding direction and saturation. A
   build that passes isn't enough.
5. Remove the matching workaround from the lab and re-run Phase 2.
6. Prepare the PR text and show it to the owner. Don't open it without approval.

## Out of scope

- Anything in Rake: its pin, `rake dev`, `rake::view!` or Rake's apps.
- Zed. Blaze's docs say its daemon needed more than 111 GB on Zed as of 2026-10-21.
- Linux and Windows, other than noting differences you happen to see.
- Comparing with Subsecond. Record the numbers; that spike has its own plan.
- Maintaining a long-term Blaze or GPUI CE fork.

## Deliverable

The lab repository, committed locally and not pushed, with one command per phase to
reproduce it. A report at `.agents/reports/blaze-gpui-bringup.md` in Rake containing:

- Environment, and every revision and version.
- The Phase 0 list, and which item each workaround removed.
- The exact cost of each workaround, particularly the loss of accessibility.
- Behavior comparison results, and any differences from cargo.
- The live-edit table, timing medians, cargo comparison and peak memory.
- Which Blaze issues this confirms, contradicts or adds to, with draft text for the owner
  to post if useful.
- A short recommendation: ready to try in Rake, ready with workarounds, or wait, and for
  which issues. Be plain about what wasn't tested.
