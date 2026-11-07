# Plan: GPUI CE Subsecond hot-patching spike

Handoff plan. The goal is to find out whether GPUI CE's experimental
`hot-patching` feature gives Rake useful hot reload, and how it would fit with
`rake dev`'s checkpointed restart.

This is a **spike**. Phases 0 and 2 measure and report. Phase 1 changes the
GPUI CE pin on a branch only. Phase 3 starts with a design note and needs owner
approval. Stop at any gate and write the report.

## Background (read once)

GPUI CE merged [#292](https://github.com/gpui-ce/gpui-ce/pull/292), which adds
an opt-in `hot-patching` feature. It uses Dioxus's Subsecond runtime.
`dx serve --hot-patch` builds and launches the app, recompiles after an edit
with real rustc, and sends the app a patch. GPUI then calls view rendering and
element layout, prepaint and paint through `subsecond::HotFn`, so those entry
points reach new code. A patch handler refreshes each window.

Read these at GPUI CE revision `bc577476a0` (main when this was written):

- `docs/pages/hot-patching.typ`: setup and limitations.
- `crates/gpui/src/app.rs` near `dioxus_devtools::connect_subsecond`.
- `crates/gpui/src/window.rs` near `subsecond::register_handler`.
- `crates/gpui/src/view.rs` and `crates/gpui/src/element.rs`: the `HotFn` calls.

[Photon](https://github.com/PhotonBrowser/photon/tree/94f0519b19c7056c52f7e25a89f3c0adb9880b75)
is a real app using it on macOS. See `crates/photon-cli/src/commands/build.rs`
(`run_hot_reload_macos`, a `dx serve` retry loop), `crates/photon-app/src/main.rs`
(re-execs from an app bundle) and `Documentation/Photon/GPUI.md`. Photon uses
Rust 1.99.0 and a vendored GPUI CE, so it doesn't prove Rake's setup works.

Known limits, from those sources:

- Native debug builds only. No Wasm and no release builds.
- GPUI CE's docs warn: "A patch can fail or crash the application."
- `dx` watches the application crate and its **direct** workspace dependencies.
  Edits in indirect workspace dependencies need a rebuild.
  An example that depends on `rake` will probably not patch edits in `rake-ui`.
- DX 0.7.10 can fail with `Cycle in workspace dependency graph` for apps launched
  inside GPUI CE's own workspace. Whether Rake's workspace triggers it is unknown.
- #292 reports patches taking **13.2 s and 4.8 s** in a small app on Wayland.
  Speed on macOS, in a Rake app, is unmeasured. Don't assume "subsecond".
- As far as we know, Subsecond does not detect struct layout changes. Confirm
  this in Phase 2. It is the main design problem for Phase 3.

## Current state in Rake

- `crates/rake`, `crates/rake-ui` and `crates/rake-native` each pin GPUI CE at
  `99109e3` (2026-08-28), which has no `hot-patching` feature.
- Moving to `bc577476a0` is **151 commits and 300 changed files**. The package was
  renamed from `gpui_ce` to `gpui-ce`, and the crate layout changed. For example,
  `gpui_elements` exists now, and Photon depends on it directly.
- Rake pins Rust 1.95.0. GPUI CE declares `rust-version = "1.95"`, and #292
  checked that the feature compiles on it, but no live patch on 1.95 is recorded.
- `dx` on this machine is **0.6.3**. The feature needs **0.7.10**.
- `scripts/dev.py` builds desktop apps through `rake bundle`, launches them and
  owns the checkpoint restart. See `build` (~line 206) and the reload path
  (~lines 400-440). The contract is in `docs/development.md`.

## Settled decisions (don't relitigate)

- **Dev only and opt-in.** The feature is never on by default. Release builds,
  `rake build`, `rake bundle` and Web never compile Subsecond or Dioxus.
- **Keep the normal restart workflow.** Default `rake dev` behavior is unchanged.
- **Struct changes use Rake's checkpointed restart.** Don't build a second
  state-preservation mechanism.
- **Don't change Rake's Rust pin** without asking the owner.
- **Don't replace the installed `dx` 0.6.3.** Install 0.7.10 into a private root.

## Before you start

- Read `AGENTS.md`, `.agents/workflow/rust/AGENTS.md`, `architecture.md`,
  `testing.md` and `stacks.md`.
- Run `git status`. Never stage, revert or reformat the owner's uncommitted work.
  Don't run plain `cargo fmt`; use `cargo fmt --all --check` or
  `rustfmt --edition 2024 <file>` on files you changed.
- The checkout may be on a feature branch. Ask the owner which base to use if it
  isn't obvious, and work on a new branch such as `spike/subsecond-hot-patch`.
- Record the machine, OS, free memory, Rust version, `dx` version and all
  revisions. Pin one GPUI CE revision and use it for the whole spike.
- Install `dx` without touching the existing one:

  ```sh
  cargo install dioxus-cli --version 0.7.10 --locked --root "$SPIKE_DIR/dx"
  ```

  Put `$SPIKE_DIR/dx/bin` first on `PATH` only for spike commands, and confirm
  with `dx --version`.

## Phase 0: Minimal app, outside Rake (report only)

Prove the feature works on this Mac before paying for a pin migration.

1. Outside this repository, create a small Cargo app. Depend on `gpui-ce` at the
   pinned revision with `hot-patching`, on Rust 1.95.0. Render a label and a color.
2. Run `dx serve --hot-patch` and record the first build time.
3. Edit the label, then the color, ten times in total. For each edit record
   save-to-visible time, whether the PID stayed the same, and whether it crashed.

If it fails on 1.95.0, repeat on 1.99.0 in this external app only, and report
both results.

**Gate 0:** stop if patches don't apply on macOS. If they only work on a newer
Rust, stop and ask the owner about the toolchain.

## Phase 1: Get the feature into Rake

Time-box this to **half a day**. The result matters more than a finished migration.

**Route A, preferred: bump the pin.**

1. Update all three pins to the same revision. Change `package = "gpui_ce"` to
   `gpui-ce`, and update `gpui_platform` and anything else the new layout needs.
2. Fix Rake's compile errors. Keep the fixes mechanical. Note API changes that
   change behavior instead of papering over them.
3. Add a `hot-patching` feature on `rake` that enables `gpui/hot-patching`. Add a
   matching feature to the examples used below. Don't add it to any `default`.

**Route B, fallback: temporary backport.** If Route A overruns, backport #292 onto
`99109e3` in a throwaway fork or local `[patch]`, and say so in the report. Route B
only answers "is it useful?" It is not something to adopt. It also needs the
same owner approval as any new fork, and must not be pushed publicly without it.

**Checks:**

- `cargo test --workspace --locked` passes, or every failure is listed and
  explained. Route A may legitimately change snapshots or rendering.
- `python3 scripts/build-web.py --example view` still builds.
- With the feature off, `cargo tree -p rake-view -e normal | grep -E 'subsecond|dioxus'`
  finds nothing. Do the same for a release build's features.

**Gate 1:** if neither route works within the time-box, stop and report what
blocks it.

## Phase 2: Measure real Rake apps (report only)

Run the apps under `dx serve --hot-patch` directly, without `rake dev`.

Use `rake-view` first, then `rake-notes`, which is a larger app with real state.
If `dx` reports `Cycle in workspace dependency graph` or similar, try a separate
consumer app outside the workspace that uses path dependencies on `rake`, like
`tests/renamed-facade`, and record the difference.

For each edit below record: outcome (patched, visibly unchanged, rebuild needed,
crash, wrong behavior), save-to-visible time, whether the PID stayed the same,
and whether in-memory state survived. Restore the source after each case.

| # | Edit | Expected | Why it matters |
| --- | --- | --- | --- |
| 1 | Text inside `rake::view!` | Patched | The main case |
| 2 | `class="..."` value | Patched | The main case |
| 3 | Helper function called from `render` | Patched | Same crate |
| 4 | New helper function, then call it | Patched? | New symbol |
| 5 | Add an element with new closures to `view!` | Patched? | New closure types |
| 6 | Body of an `on_click` closure | Unknown | Is the old closure kept until re-render? |
| 7 | Body of a managed task or `cx.spawn` future | Old code? | Not entered through `HotFn` |
| 8 | `#[rake::instrument]` function body | Unknown | Macro-wrapped |
| 9 | Route mount code in `RouteViews::mount` | Old code? | Runs only at mount |
| 10 | Edit a direct workspace dependency (`crates/rake`) | Patched? | Framework change |
| 11 | Edit an indirect dependency (`crates/rake-ui`) | Rebuild | Documented limit |
| 12 | Add a field to a view struct | **Unsafe** | Does anything detect or refuse this? |
| 13 | Change a field's type or reorder fields | **Unsafe** | Silent corruption or crash? |
| 14 | A compile error, then its fix | App keeps running | Recovery |
| 15 | Ten quick saves in a row | All land | Coalescing |

**Baseline:** time the same text edit under today's
`rake dev --target desktop --package rake-view` (save to the restored window),
and record the cold build time for both.

For cases 12 and 13, watch for silent bad behavior as well as crashes. Run them
last, in a disposable process, and don't count a run that merely looks fine as
safe. Note what the docs and Subsecond's source say about layout changes.

Also note: whether the Rake inspector and `RAKE_DEV_CONTROL` still work under
`dx`; whether windows, tabs and focus stay intact after a patch; whether macOS
app identity matters for these examples (Photon needed its bundle re-exec; the
native component features may too); and memory use.

**Gate 2: report to the owner and stop.** Proposed bar for continuing, which the
owner may change: cases 1-3 patch at least 9 of 10 times, median save-to-visible
under 3 s on `rake-notes` and clearly faster than the restart baseline, and no
crashes in cases 1-5.

## Phase 3: Fit with `rake dev` (design note first)

Only after the owner approves at Gate 2. Write a short design note answering
these questions, and get approval before writing prototype code:

1. **How does Rake learn an edit is unsafe?** This is the crux. If Subsecond
   doesn't refuse layout changes, nothing triggers the checkpointed restart and the
   app may run corrupted. Compare at least: hashing type definitions before and
   after each edit (Blaze's approach, in its `docs/live.md`, "What is refused"); a
   compiler-based check; and an explicit restart command, with a clear warning.
   State what each one misses. A false "safe" is the failure that matters most.
2. **Who owns the app process?** `dx` launches the app, while `scripts/dev.py` owns
   its process group, shutdown and status. Decide who watches files too, since two
   watchers could race. Cover Ctrl-C and crash cleanup, and how a crashed app
   restarts, as Photon's retry loop does.
3. **How does a restart work under `dx`?** Prepare, capture the checkpoint, stop,
   relaunch with the checkpoint. Measure what the relaunch costs: a full `dx` build
   or a reuse. Check `RAKE_DEV_CONTROL` and the control directory reach the app.
4. **Does a patch go through the reload boundary?** Should patches bump
   `GenerationId` through `NativeCodeReplacement` and `finish_reload`, or bypass it?
   The full boundary cancels managed work and detaches route outlets, which is likely
   too heavy per edit. Skipping it means the generation won't reflect code changes.
   Propose an option and let the owner decide.
5. **How does a developer see what happened?** Patched, restarted, or refused,
   in the terminal, `--json` output and the inspector.

If approved, prototype `rake dev --hot-patch` for desktop only. Reject it with
`--target web` and `--release`, the way `--native-reloader` is validated in
`scripts/dev.py`. Test argument validation and decision logic in
`scripts/test-supervisor.py` without needing `dx`. Don't add tests that need a live
`dx` to CI.

## Side note: Blaze recheck

Blaze's patching reaches every caller, without `HotFn`, so it remains the better
long-term engine if it can build GPUI. The first spike stopped at Gate 1; see
`.agents/reports/blaze-live-spike.md`.

Spend at most 15 minutes, at the end, checking from GitHub only. Don't reinstall
Blaze for this. Record whether these have closed:
[#1260](https://github.com/indexable-inc/blaze/issues/1260) (`pathfinder_simd`,
`zune-jpeg`), [#1259](https://github.com/indexable-inc/blaze/issues/1259)
(`accesskit_macos`) and [#1303](https://github.com/indexable-inc/blaze/issues/1303),
whether a stable release newer than `v0.20261027.0` exists, and whether
[#1349](https://github.com/indexable-inc/blaze/issues/1349)'s fix covers
`clang-sys` 1.9.1, which Rake locks. Recommend a retest only if all of those are
resolved.

## Out of scope

- Adopting the pin bump on `main`. Any merge is a separate decision after this spike.
- Web hot patching, and Linux or Windows support beyond noting what you saw.
- Corgi or other build-speed tools.
- Rite changes.
- Fixing bugs in Dioxus, Subsecond or GPUI CE. Report them to the owner, and don't
  open upstream issues or PRs without asking.

## Deliverable

A report at `.agents/reports/subsecond-hot-patch-spike.md` containing:

- Environment and every revision, including which route Phase 1 used.
- Phase 0 results.
- Phase 1 results: size of the migration, behavior changes, test results, and the
  feature-off dependency check.
- Phase 2: the full edit table for each app, measured medians next to the restart
  baseline, and what happened in the unsafe cases.
- The design note, and prototype status if Phase 3 ran.
- The Blaze recheck.
- A one-paragraph recommendation: adopt, adopt only with a safety check, wait, or
  drop. Be plain about what was not tested.

Don't push, open a PR or merge without asking the owner. Keep raw logs in the
report's folder only if they are small.
