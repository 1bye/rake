# Plan: Blaze compile spike and live-edit prototype

Handoff plan. The goal is to find out whether
[Blaze](https://github.com/indexable-inc/blaze) can build Rake's GPUI apps, and
if it can, prototype hot reload in native `rake dev` on top of Blaze's
`blaze-live` crate.

This is a **spike**. Phases 1 and 2 only measure and report. Phases 3 and 4
are prototype code on a branch, and they start only if Phase 2 succeeds and the
owner agrees. Stop at any gate below and write the report.

## Background (read once)

Blaze replaces cargo for builds. It keeps the project compiled in memory
(rust-analyzer MIR plus Cranelift), and after an edit compiles only the changed
functions and splices them into the existing executable. A program that links
`blaze-live` and calls `Live::start()` takes those patches while it runs.
`Live::poll()` applies the newest one and returns one of these:

- `Outcome::Applied { functions, statics, .. }`: changed functions now jump to
  new code. A function already running finishes on its old code.
- `Outcome::Restart(reason)`: a type definition changed, or a static's first
  value did. The program must be started again.
- `Outcome::Declined(reason)`: the program runs on unchanged.

Docs to read in the Blaze repo: `README.md`, `docs/usage.md` ("What works and
what doesn't yet"), `docs/live.md` (whole file: what is refused, statics,
signing, the "In den" section on integrating with an existing app), and
`crates/blaze-live/src/lib.rs`. `demo/life/src/window.rs` around lines 313 and
345 shows `Live::start` inside a windowed app.

Rake already has the matching slot, unused:

- `ReloadMechanism::NativeCodeReplacement` in `crates/rake/src/reload/mod.rs`.
- `finish_reload(id, applied)` in `crates/rake/src/router/runtime/reload.rs`
  around line 246.
- `rake dev --native-reloader PATH` in `scripts/dev.py`: the build is at line
  ~197 (`cargo build --message-format=json-render-diagnostics`), and the reload
  path is lines ~340-400.
- The contract is in `docs/development.md`, sections "Shared runtime boundary"
  and "Native adapter".

The two models differ, and that is the main design point for Phase 4. Rake's
`--native-reloader` is a separate executable run by the supervisor that must
"replace code" and exit 0. Blaze's patch is applied **inside** the app by
`Live::poll()`, and the Blaze daemon (not the supervisor) produces it.

## Settled decisions (don't relitigate)

- **Dev only, native only.** Release builds, `rake build`, `rake bundle` and
  the Web target never depend on Blaze or `blaze-live`. Web keeps full page
  reload.
- **Opt-in.** Nothing changes for users who don't pass the new flag or enable
  the new feature. The default `native_restart` path stays as it is.
- **Cargo stays the source of truth.** Blaze's borrow checker is incomplete.
  `cargo check`, the workspace tests and CI remain the final word.
- **`Restart` falls back to Rake's checkpointed restart.** Don't invent a
  second state-preservation mechanism.
- **Poll on the UI thread at a frame boundary.** Do not use
  `Live::apply_as_they_come`: it patches mid-frame.
- **Don't vendor Blaze.** Depend on `blaze-live` by version from crates.io or
  by git rev, whichever exists and builds.
- **Primary platform: macOS on Apple silicon.** Linux x86-64 is a bonus, not a
  requirement.

## Before you start

- Read `AGENTS.md`, `.agents/workflow/rust/AGENTS.md`, `architecture.md`,
  `testing.md` and `stacks.md`.
- Run `git status`. The owner may have uncommitted work, including other files
  in `.agents/plans/`. Never stage, revert or reformat it. Don't run plain
  `cargo fmt`; use `cargo fmt --all --check` or `rustfmt --edition 2024 <file>`
  on files you changed.
- Work on a new branch, for example `spike/blaze-live`. Don't commit to `main`.
- Rake pins Rust 1.95.0 (`rust-toolchain.toml`) and GPUI CE rev `99109e3`
  (`crates/rake/Cargo.toml`). Blaze ignores the pin and builds with its own
  nightly (`blaze-2026-10-27` or newer). Don't change Rake's pin.
- **Ask the owner before installing Blaze.** The installer is
  `curl -fsSL https://ix.dev/blaze | sh`, which downloads a 125-160 MB
  toolchain. `docs/install.md` in the Blaze repo lists alternatives (crates.io,
  Nix). Record the Blaze version and commit you used.
- Blaze trades memory for speed (several GB). Note free memory before starting.

## Phase 1: Does Rake compile under Blaze? (report only)

From the workspace root, for each package in this order:
`rake-foundation`, `rake-view`, `rake-routing`, `rake-notes`, `rake-inspector`.

1. `blaze build -p <pkg>`, and record wall time for the cold build.
2. If it fails, record the **first** error verbatim and which crate it is in
   (GPUI CE, `objc2-*`, a build script, a proc macro, Rake itself). Don't try
   to fix GPUI or third-party crates. A small Rake-side workaround is fine if
   it is obviously correct; note it.
3. If it builds, `blaze run -p <pkg>` and check by hand that the window opens
   and behaves as under `cargo run`. Take a screenshot if you can.
4. Edit one function body (for `rake-view`, change a string or class inside
   `rake::view!` in `examples/view/src`), rebuild, and record the time and
   whether the `Finished` line says `[blaze]`.
5. Also run `cargo test --workspace --locked` once (with cargo) to confirm the
   tree is green, so later regressions are attributable.

**Gate 1:** if no GPUI example builds and runs, stop. Write the report with the
errors and which upstream (Blaze or GPUI CE) would need a fix. Don't open
issues upstream without asking the owner.

## Phase 2: Live edits in a throwaway harness (report only)

Goal: prove `blaze-live` patches a GPUI window before touching Rake.

1. Outside the workspace (or as an unlisted example that is never added to
   `members`), make a minimal binary on `rake` that renders a `rake::view!`
   with a label and a color.
2. Add `blaze-live`. At startup call `Live::start()` and log the result. Poll
   it from a foreground timer every ~16-30 ms. On `Applied`, call
   `cx.refresh_windows()` (`App::refresh_windows`, gpui `app.rs:1069` at rev
   `99109e3`) so GPUI calls `render()` again, and log the outcome.
3. Run it with `blaze run`. In another terminal, edit the label text, then the
   color, then a `class="..."` value, then add a field to the view struct.
4. For each edit, record: the outcome variant, the reason string for
   `Restart`/`Declined`, time from save to visible change, and whether the
   window repainted without other input.
5. Note macOS signing issues if any (`docs/live.md`, "Signing").

**Gate 2:** if body and `view!` markup edits don't come back `Applied` and
visible, stop and report. If they do, show the owner the numbers and ask before
Phase 3.

## Phase 3: In-app integration in `rake` (prototype)

1. Add a `live` cargo feature to `crates/rake`, native targets only, enabling
   an optional `blaze-live` dependency. Never enabled by default and never by
   the examples' default features.
2. Start `Live` only when the feature is on **and** `RAKE_DEV_CONTROL=1` **and**
   a new `RAKE_LIVE=blaze` is set. Look at `crates/rake/src/reload/host.rs` and
   `crates/rake/src/control/mod.rs` for how the existing environment checks are
   structured, and follow that style.
3. Poll on the UI thread, at a frame boundary, from the runner in
   `crates/rake/src/application.rs` (around `run`, line ~216). Keep the poll
   cheap when nothing arrived.
4. On `Applied`: refresh all windows, and publish a runtime event (for example
   `live_patch_applied { functions, statics, took }`) through the existing
   event channel so the devtools console and `--json` output can show it.
5. On `Restart(reason)`: publish an event asking the supervisor for a restart,
   carrying the reason. Don't restart from inside the app.
6. On `Declined(reason)`: log at WARN and publish an event. Keep running.
7. Decide, and write down why, whether an `Applied` patch should go through
   Rake's reload boundary (`prepare_reload` with `NativeCodeReplacement`, then
   `finish_reload(applied=true)` to bump `GenerationId`) or bypass it. The full
   boundary cancels managed futures and detaches the route outlet, which is
   probably too heavy per keystroke; but skipping it means `GenerationId` won't
   reflect code changes. Propose the option; don't silently pick one.
8. Tests: unit-test the outcome-to-event mapping without Blaze (inject a fake
   outcome source). Don't add tests that need the Blaze daemon to CI.

## Phase 4: Supervisor integration (prototype)

1. Add `rake dev --compiler blaze` (desktop only; reject it with `--target web`,
   following how `--native-reloader` is validated near `scripts/dev.py:556`).
   It sets `RAKE_LIVE=blaze` for the launched app and builds with
   `blaze build` instead of `cargo build`.
2. Find out what `blaze build` prints for a patch versus a full relink (Blaze
   answers like `ok patch=... functions=N ms=N` over its socket; check what the
   CLI prints and whether it supports a JSON message format). Use that to
   decide:
   - patch: don't relaunch; the app picks it up itself.
   - relink, or a `Restart` event from the app: use the existing checkpointed
     `native_restart` path.
3. Launch exactly the executable Blaze watches. `docs/live.md` warns that the
   copy in `target/debug` is only updated by a build through cargo, and the app
   is keyed by its `LC_UUID`. Verify the running app actually receives patches
   after a relink-and-restart cycle, not only on the first run.
4. Keep the build-failure behavior: a failed edit leaves the old app running.
5. `rake doctor` should report whether `blaze` is on PATH and its version.
6. Extend `scripts/test-supervisor.py` only with tests that don't need Blaze
   installed (for example, argument validation and the patch-versus-relink
   decision given recorded output).

## Out of scope

- Editor integration through `blaze_live::edit` (unsaved-buffer builds).
- Blaze's tiering (`BLAZE_TIER`); leave it at its default, or set
  `BLAZE_TIER=off` if it adds noise, and say which.
- Wasm or controlled Wasm module swapping.
- Making `rake build` or `rake bundle` use Blaze.
- Fixing bugs in Blaze or GPUI CE.

## Deliverable

A report at `.agents/reports/blaze-live-spike.md` containing:

- Blaze version/commit, machine, OS, free memory, Rake commit.
- Phase 1 table: package, cold build time, builds?, runs correctly?, edit
  rebuild time, first error if any.
- Phase 2 table: edit kind, outcome, reason, save-to-pixels time.
- If Phases 3-4 were done: the branch name, the design choice from Phase 3
  step 7 with reasoning, what works end to end, and a short list of known gaps.
- A one-paragraph recommendation: adopt, wait for Blaze to mature (and on
  what), or drop.

Don't open a PR or push without asking the owner.
