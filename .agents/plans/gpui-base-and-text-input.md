# Plan: move to `gpui-pre` and `gpui-base`, adopt its text input

Handoff plan. The goal is to run Rake on the GPUI that GPUI Kit uses, build
`rake-ui` on `gpui-base`, and replace the hand-written text inputs with Base's.

**The main finding:** Rake doesn't need to write a text input. `gpui_base::input`
already has `Input`, `Textarea` and `Editor`, on a shared engine of about 26,000
lines with IME composition, masking, undo and accessibility roles. This plan is
mostly a migration and a qualification effort, not an editor project.

Work the phases in order, with one reviewable change per phase. Phase 0 is a
spike that changes nothing on the working branch. Stop at any gate and report.

## Settled decisions (don't relitigate)

Decided by the owner on 2026-11-04:

- Rake stays GPUI-first. NSH was declined as a renderer.
- `rake-ui` builds on `gpui-base`. GPUI Kit is an optional first-party feature.
- Rake's theme becomes compatible with Kit's, not the reverse.
- GPUI Fast stays possible, but this plan doesn't ship it.
- Don't promise arbitrary GPUI revisions. See `docs/future/gpui-variations.md`.

Decisions this plan makes. Raise them with the owner if the evidence disagrees:

- **Replace CE, don't support both.** One pinned GPUI at a time. Multi-backend
  support remains deferred.
- **No new `view!` grammar for inputs.** Base's `Input` and `Textarea` are
  `RenderOnce` components with `Input::new(&Entity<InputState>)`, and `view!`
  already has typed component tags and `<view entity=...>`. Add a macro tag only
  if real use shows the need.
- **Keep Rake's overlays for now.** `rake::overlay` stays. Comparing it with Base's
  popover and dialog is separate work.

**Out of scope:** the GPUI Kit feature, other Base widgets, forms and validation,
GPUI Fast, `rake new`, and any reload work.

## Facts checked on 2026-11-04

| Fact | Evidence |
| --- | --- |
| Base, Component and Kit are 0.9.0, released 2026-11-02 | crates.io |
| Base pins `gpui-pre` exactly, at `=0.3.11` | `gpui-base-0.9.0/Cargo.toml` |
| Rake pins `gpui_ce` at `99109e3` in three crates | `rake`, `rake-ui` and `rake-native` manifests |
| 148 of 151 `gpui::` paths Rake uses have a same-named item in `gpui-pre` | Name search only, signatures not compared |
| Missing names: `rgb_to_hsla`, `http_client::HttpResponse`, `set_frame_trace_enabled` | Used in macros, tests, `application.rs` and `inspection/recording.rs` |
| The APIs inspection depends on exist in `gpui-pre`: `a11y_role`, `write_a11y_info`, `aria_*` | Name search only |
| `EntityInputHandler` grew defaulted methods in `gpui-pre`, including `paste` | Source diff against CE |
| The two GPUIs have diverged a lot | `window.rs` has 4,787 differing lines, `div.rs` 1,150 |
| `gpui-pre` has its own `hot-reload` feature using Subsecond | `gpui-pre-0.3.11/Cargo.toml` |
| Base has no feature to drop its editor stack | It always depends on `lsp-types`, `markdown` and `html5ever` |
| Kit calls Wasm "showcase only" and Base's Wasm example uses nightly | [GPUI Kit WebAssembly guide](https://gpui-kit.com/docs/webassembly/) |

**Not yet known, and Phase 0 must answer:** whether any of this builds on Rust
1.95, whether Wasm builds on stable, and whether Rake's `wasm-bindgen` 0.2.129
works with `gpui-pre-web`. Kit's guide names 0.2.121.

The last row matters most. Rake supports Web as a product target, on stable Rust.
Kit doesn't claim that, so Rake would be first to rely on it.

## Before you start

- Read `AGENTS.md`, `.agents/workflow/rust/AGENTS.md`, `architecture.md`,
  `testing.md` and `rustdoc.md`.
- Run `git status`. The owner has large uncommitted changes, including
  `examples/notes/src/{editor,view}.rs`. **Phases 1 to 5 shouldn't start until
  those are committed**, because Phase 5 rewrites the same files. Never stage,
  revert or reformat them. Use `cargo fmt --all --check`, or `rustfmt --edition 2024`
  on the files you changed.
- Do Phase 0 in a separate worktree or scratch project.

## Phase 0: compatibility spike (report only)

Build a minimal app on `gpui-base` and `gpui-pre-platform`, with one `Input` and
one `Textarea`. Then try the pin swap on a throwaway branch to size Phase 2.

Record each result in `.agents/reports/gpui-base-spike.md`:

| Gate | Pass condition | If it fails |
| --- | --- | --- |
| Native on Rust 1.95 | Builds and runs on macOS | Report the minimum version; the owner decides on raising the pin |
| Wasm on stable | `wasm32-unknown-unknown` builds on stable | **Stop.** Nightly for Web is an owner decision |
| `wasm-bindgen` | One locked version satisfies Rake and `gpui-pre-web` | Report the conflict |
| Browser run | In Chrome, the app renders and typing works, including IME composition | **Stop.** Web would regress |
| Wasm size | Record compressed size against today's `rake-notes` | Over +25%: the owner decides, possibly asking upstream for a feature split |
| Native compile time | Record clean and incremental times against today | Report only |
| Migration size | Count of compile errors by crate after the pin swap | Over about 300 or requiring redesign: **stop** and re-plan |

Also answer:

- Does `Input` work without Base's `Root`? The input module has no direct `Root`
  reference, but Base's popovers and dialogs may need it.
- Does `gpui-pre` have a frame-trace replacement for `set_frame_trace_enabled`?
  Devtools recording depends on it.
- Which fixes do the two GPUIs differ on that Rake relies on? Check the commits
  behind Rake's recent native and accessibility fixes.

**Owner gate:** review the report before Phase 1.

## Phase 1: centralize the pin (no behavior change)

Move `gpui` and `gpui_platform` to `[workspace.dependencies]`, including any
crates that declare them indirectly. Handle `examples/inspector` separately, since
it's outside the workspace. The next phase should then change the pin in one place.

Verify with `cargo check --workspace --all-targets --locked`, with no other change
to `Cargo.lock`.

## Phase 2: replace GPUI CE with `gpui-pre`

Keep the `gpui` dependency alias, so `rake::gpui` and generated macro paths stay
valid. Don't add a compatibility layer.

- Fix the known missing names first, then work through the compiler errors crate by
  crate: `rake-ui`, `rake-macros` snapshots, `rake`, `rake-native`, then the examples.
- Review macro snapshot changes by hand. Don't accept them in bulk.
- If `set_frame_trace_enabled` has no replacement, report recording as a reduced
  capability through devtools. Don't fake the data.
- Check Linux and Windows too. Base has `ashpd` and `windows` dependencies, and the
  platform feature set differs from CE's.
- Update `README.md`, `docs/release-readiness.md` and
  `docs/future/gpui-variations.md`, and add migration notes.
- Retarget `.agents/plans/subsecond-hot-patch-spike.md`, which assumes CE, and note
  that the Blaze report tested CE.

**Done when** all of `check.yml` passes, plus the native smoke and layout scripts,
`test-elements.py`, the CDP tests, the AppKit tests in `rake-native`, and the
`browser` group of `check-readiness.py`. All existing behavior must hold. This phase
adds no features.

## Phase 3: add `gpui-base` to `rake-ui` and bridge the theme

- Add `gpui-base` to `rake-ui` and call `gpui_base::init` once during `App` setup,
  only when `experimental-view` is enabled. Non-UI builds must not gain the dependency.
- Rake's themes are the app's own struct, and Base uses a global `Theme` with
  `SemanticThemeTokens`. Add an explicit, optional conversion: an app implements a
  small trait mapping its theme to Base's tokens, and `rake::theme::install` then
  sets Base's global too. Installing without that mapping leaves Base's defaults.
- Check that a theme change reaches Base components and that Base's fonts resolve on
  Web. Kit's guide warns that themes can name desktop-only font families.
- Keep `rake::theme`'s existing API working.

**Done when** a test shows a theme change repainting a Base `Input` with the mapped
colors, and `cargo check -p rake --no-default-features` has no Base in its tree.

## Phase 4: expose inputs from Rake

- Expose `rake::ui::input` as a thin re-export of `Input`, `InputState`, `Textarea`,
  `TextareaState` and `InputEvent`, behind `experimental-view`. Don't wrap them unless
  Phase 5 shows a real need. Document that these types follow Base's versioning.
- **Inspection privacy:** the protocol promises snapshots never contain editable text
  or password values. Add a test covering Base's inputs, including masked ones, across
  the CDP DOM, accessibility and storage views.
- **Agent input:** extend `scripts/test-agent-cdp.mjs` so semantic queries, typing and
  actionability work against a Base `Input`.
- Add an example to `examples/view` and document it in `docs/experimental-view.md`.

## Phase 5: migrate the hand-written inputs

Go from smallest to largest, one commit each:

1. `examples/inspector/src/input.rs` (390 lines), the filter field.
2. Notes search.
3. Notes title.
4. Notes body, the hardest. Check autosave, crash recovery, word and character counts,
   and `set_text(..., reset, ...)`, which replaces text when switching notes and must
   not leave undo history that crosses notes.

Delete `examples/notes/src/editor.rs` once nothing uses it, moving its Unicode tests to
the Notes integration level first. `InputEditsEvent` may suit autosave better than
reading state on `Change`.

**Leave `examples/agent/src/main.rs` alone.** It's a test fixture for native input
through CDP, and it needs its own low-level handler.

**Done when** `test-notes.py`, `test-notes-view.py`, `test-notes-storage.mjs` and the
Notes browser procedure pass, and the three `notes.*` mounts still appear in inspection.

## Phase 6: real-host qualification

Run the text input gate from `docs/qualification.md` against the migrated Notes: Latin,
CJK IME composition, emoji and combining characters, selection, clipboard, undo, dead
keys and layout changes. Test macOS, Windows, Linux (X11 and Wayland separately) and
Chromium. On Web, test focus, Tab order and paste, since input goes through a hidden
HTML field.

Also run VoiceOver and NVDA over the same fields. Record OS, browser and versions in
`.agents/reports/gpui-base-qualification.md`. Failures become upstream issues or Rake
fixes. **Don't record anything you didn't run as passing.**

## Risks

| Risk | Mitigation |
| --- | --- |
| Web needs nightly or regresses, since Kit doesn't target full Web apps | Phase 0 stops the plan before any commitment |
| Wasm size grows from the always-on editor stack | Measure in Phase 0, then ask upstream for a feature split |
| The exact `=0.3.11` pin means Rake moves only when Base does | Upgrade together, one change per version, with the Phase 2 checks |
| `gpui-pre` is a pre-release family updated often | Pin exactly with a lockfile; adopt upgrades on purpose |
| Lost GPUI CE behavior that Rake's native fixes depended on | Phase 0's fix audit, then the AppKit tests in Phase 2 |
| Re-exports tie Rake's API to Base's churn | Keep them under `experimental-view`, with no stability promise before beta |

## Reporting

For each phase, include the diff summary, each verification command's result, measured
numbers and anything skipped. Don't push. Commit only when the owner asks.

Sources: [gpui-base](https://crates.io/crates/gpui-base),
[gpui-pre](https://crates.io/crates/gpui-pre),
[GPUI Kit repository](https://github.com/longbridge/gpui-kit).
