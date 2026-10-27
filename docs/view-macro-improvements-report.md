# View macro improvement report

Implemented Phases 0–8 of `.agents/plans/view-macro-improvements.md` in order,
with one commit per phase. The owner confirmed all proposed defaults: trybuild,
text-only headings, canonical events with deprecated aliases, and the proposed
boolean/shorthand/default-field syntax. Baseline: `61a8f5c`.

## Phase results

| Phase | Commit | Result and reviewed expansion changes |
| --- | --- | --- |
| 0 | `92a1060` | Added 13 baseline expansion snapshots and 18 compiler diagnostic fixtures; macro behavior unchanged. Reviewed each diagnostic's offending-token span. |
| 1 | `e70326a` | Resolved tags and split builder/attribute/focus/child expansion. All baseline snapshots and stderr remained identical. Added the requested duplicate-ID hint. |
| 2 | `7bad146` | Headings compose inline text and expose its full accessible name; explicit labels win. Only the heading snapshot changed. No in-repo heading required migration. |
| 3 | `9c6fb8c` | Combined independent errors, recovered attribute boundaries, and retained valid builder/listener types. Existing snapshots and stderr stayed unchanged. New fixtures prove three diagnostics and expression/listener type errors beside macro errors. |
| 4 | `826fe28` | Added checked utilities and conditional classes. Existing snapshots stayed identical; a conditional-class snapshot was added. Documentation/table coverage and real GPUI compilation cover every supported utility. Inspector styles were not migrated. |
| 5 | `1739f1e` | Added canonical events, deprecated aliases and tooltip support. Migrated consumers without deprecation warnings. Existing valid expansions stayed identical; a complete event snapshot and alias-warning fixture were added. |
| 6 | `f8b2f0e` | Added semantic containers, labelled/decorative SVGs and rules. Existing snapshots stayed identical; a semantic-elements snapshot was added. Raster/vector accessibility delegation shares the existing image wrapper. |
| 7 | `01545e2` | Added guarded match arms, bare booleans, prop shorthand and final component defaults. Existing valid snapshots stayed identical; a syntax snapshot was added. Added ten rejection fixtures, evaluation-order tests and renamed-facade native/Wasm coverage. |
| 8 | This documentation commit | Added formatting examples and guidance; reconciled rustdoc, the guide, identity rules, events and current limits. No expansion changes. |

## Files and consumer migrations

- Phase 0: `Cargo.lock`, `crates/rake/Cargo.toml`,
  `crates/rake-macros/src/view/tests.rs` and `view/snapshots/`,
  `crates/rake/tests/view_ui.rs`, `tests/ui/view/`, and
  `docs/experimental-view.md`.
- Phase 1: `crates/rake-macros/src/view.rs`, `view/tag.rs`,
  `view/component.rs`, and `crates/rake/src/view.rs`.
- Phase 2: macro rustdoc, `view.rs`, `view/tag.rs`, `view/paragraph.rs`,
  snapshot tests and the heading snapshot; runtime `view/paragraph.rs`;
  `view_paragraphs.rs`, frame paragraph tests, heading rejection fixtures and
  the guide.
- Phase 3: macro rustdoc, parser, new `view/diagnostics.rs`, inline utility
  expansion, macro tests, four new compiler-fixture pairs and the guide.
- Phase 4: macro rustdoc/parser, new `view/classes.rs`, diagnostics/inline
  expansion, snapshot/documentation tests, real utility-construction tests,
  three new compiler-fixture pairs and the guide.
- Phase 5: macro rustdoc/parser and new `view/events.rs`, event snapshot,
  hidden alias constants in `crates/rake/src/lib.rs`, construction/component/
  keyboard/overlay tests, new pointer frame tests, compiler fixtures, both
  experimental guides, `examples/view/src/{lib,documents}.rs`,
  `tests/renamed-facade/src/lib.rs`, and syntax-only substitutions in
  `examples/inspector/src/view.rs`.
- Phase 6: macro rustdoc/parser/tag/diagnostic tables, semantic snapshot,
  the runtime image/SVG wrapper and facade exports, semantic construction
  tests, four new compiler-fixture pairs and the guide.
- Phase 7: macro rustdoc/parser/component/paragraph/diagnostic expansion,
  syntax snapshot and unit expectations, construction/component/paragraph
  tests, ten new compiler-fixture pairs, the renamed facade and the guide.
- Phase 8: macro rustdoc, `docs/experimental-view.md`, and this report.

The owner's pre-existing edits in `docs/rake-inspector.md` and
`examples/inspector/src/{appearance,input,tree,view}.rs` were preserved.
Only canonical event-name substitutions from the committed inspector view were
staged in Phase 5; the owner's broader edits remained outside the commit. The
supplied plan remains untouched and untracked.

## Verification

All seven plan checks passed for every phase, using `--locked` and, after the
approved dependency fetch, `--offline`:

```sh
cargo test -p rake-macros --locked
cargo test -p rake --features experimental-view --test view --test view_components --test view_paragraphs --test view_ui --locked
cargo test -p rake --features view-test-support --test view_state --locked
cargo test -p rake --features experimental-view --doc --locked
cargo check -p rake-view -p rake-inspector -p renamed-facade --all-targets --locked
cargo check -p rake-view -p renamed-facade --lib --target wasm32-unknown-unknown --locked
cargo clippy -p rake-macros -p rake --features experimental-view --all-targets --locked
```

Additional final checks passed:

- Full affected-package suite: `rake`, `rake-macros`, `rake-view` and
  `renamed-facade`, with `rake/view-test-support`: 209 tests across the Cargo
  suites, including the UI harness; its 42 compiler fixtures also passed.
- Strict Clippy with `-D warnings` for those four packages and all targets.
- Repository Dylint spacing checks, rustfmt checks and `git diff --check`.
- `cargo check -p rake --no-default-features`.
- Rustdoc builds with `RUSTDOCFLAGS='-D warnings'`.
- Direct rustdoc execution of `docs/experimental-view.md`: all ten Rust
  examples, including both formatting examples, passed against the built facade.
- A direct rust-analyzer LSP probe completed `Demo.count` as `usize` inside
  `cx.listener` beside an invalid utility. This used an in-memory document;
  no source file was changed. This replaced an interactive editor check.

The initial broader suite hit sandbox loopback restrictions in existing HTTP
tests. It passed when rerun with loopback access. Strict Clippy also caught the
renamed fixture's item ordering; that was fixed in the Phase 7 commit before
finishing Phase 8. Dylint emitted the existing upstream `block 0.1.6` future
compatibility notice; there are no remaining project lint diagnostics.

## Verification limits and API boundaries

This run did not launch a native app, browser, or screen reader. Historical native
smoke-test descriptions in the guide refer to earlier iterations. Native and
Wasm checks establish compilation; GPUI frame tests use TestPlatform's
window/text/GPU doubles. Heading accessibility checks inspect real AccessKit
node properties during render because TestPlatform has no active accessibility
adapter.

Headings intentionally reject widget/layout content. Inline conditional classes
remain unsupported. SVGs use the documented restricted leaf attributes and no
macro identity; wrap them for IDs and interaction. The pinned AccessKit version
has no separator role, so `hr` has no invented role. GPUI's outside mouse-down
callback covers any button, while outside mouse-up uses the left button in this
API. A dedicated macro formatter remains a separate follow-up.
