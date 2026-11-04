# GPUI pre migration verification

Worktree `/private/tmp/rake-gpui-pre-kit-base`, branch
`codex/gpui-pre-kit-base`, based on `main` at `5fc0f9d`.
No merge from `codex/flaky-cdp-native-cleanups`.

## Phase 1

Rust 1.98.0 is the minimum stable toolchain for GPUI pre 0.3.11:
`Result::map_or_default`, used by its macOS window implementation, stabilized
in 1.98. Rake's MSRV and active CI pins follow it.

The platform dependency disables default features. Its Web `multithreaded`
feature pulls `wasm_thread`, which needs nightly; the single-thread path builds
and runs on stable. Browser background work now runs cooperatively on the main
thread. CPU-heavy work can delay input/rendering; no equivalent CPU stress
benchmark was run. Hidden-tab autosave still worked (2.35 seconds in the
observed run). Native platform features are retained explicitly.

GPUI pre starts Web text with an empty font database. Rake installs the same
unmodified IBM Plex Sans/Lilex fonts previously embedded by CE, with OFL notices
copied into Web output. This restores default text rendering without vendoring
GPUI source. Native builds do not embed these assets.

Other adaptations preserve scroll event syntax, color inspection (both old HSL
and new hex snapshots), paragraph wrapping, HTTP methods/headers/body/status,
redirect policy and timeout, and frame recording through the broader profiler
API. GPUI `KeyBinding::new` accepts explicit static modifiers; dynamic or
`secondary-` shortcuts use `load`/`parse_with_os`. The removed notes editor
`letter_spacing: None` field had no active effect.

GPUI pre defers closed-window registry removal to a foreground task. The
AppKit test still checks native detachment and accessibility removal immediately;
only the GPUI registry-removal assertion moves to its next source-window frame.
A retained NSWindowWillClose observer preserves immediate native teardown.
Trybuild snapshots reflect changed GPUI color trait diagnostics; the rejected
fixtures and their assertions are unchanged. Two test iterator spellings follow
Rust 1.98 Clippy. Spacing-only changes in native accessibility are required by
the existing repository lint.

## Real browser checks

macOS 26.6.2 arm64, Google Chrome 156.0.8078.80, plus Codex's Chromium browser.
Built notes with stable 1.98.0 and wasm-bindgen 0.2.129, served from loopback.
Used real canvas/pointer/keyboard/clipboard input for title/body editing,
paragraphs, selection, Undo/Redo, Unicode (café, CJK, emoji and a ZWJ sequence),
and an exact 80-line paste with visible caret scrolling. The stored 80-line
body matched the clipboard exactly. Actual rendered default fonts were checked. The view example still displays
missing-glyph boxes for its CJK preview and command-symbol hint with these
Latin fonts; Unicode storage equality does not establish glyph coverage.
Applications needing those glyphs must load suitable fonts. No CE comparison
of that glyph coverage was performed.

Registered notes control actions and browser storage inspection also verified:

- main/recovery snapshots agree after successful saves and ordinary reload;
- a recovery-only checkpoint survives reload and repairs the main snapshot;
- a simulated main-storage quota failure retains recovery, and retry repairs both;
- an invalid higher-revision recovery is rejected, valid notes survive, and the
  UI reports that the damaged save was repaired;
- a genuinely hidden Chrome tab autosaves; a second tab's stale save is rejected,
  remains dirty, and reload restores the accepted library without its stale edit.

Temporary storage failure injection was removed and storage repaired. These
were isolated test notes. A forced renderer crash was blocked by browser URL
policy; **renderer-crash recovery was not verified**. Ordinary reload recovery
passed. Safari, Firefox, Windows and Linux were not run.

## Phase 1 gates

Every command from `.github/workflows/check.yml` was run, plus explicit AppKit
and workspace no-fail-fast gates. Results below are the final results after
fixing migration errors, not claims that each passed on the first attempt.
The known CDP lost-submission test flaked once during a workspace rerun. A
subsequent complete workspace run and the isolated test passed; no CDP code
was changed. The cleanup branch overlaps native accessibility and AppKit tests;
its CDP/native README changes were not merged.

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | passed |
| `cargo check --workspace --all-targets --locked` | passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | passed |
| `cargo clippy -p rake --features native-components --all-targets --locked -- -D warnings` | passed |
| `cargo clippy -p rake-native --features native-tests --all-targets --locked -- -D warnings` | passed |
| `cargo test -p rake-native --doc --locked` | passed |
| `cargo clippy -p rake-client -p rake-http -p rake-protocol -p rake-diagnostics --lib --locked -- -D warnings -D clippy::missing_errors_doc -D clippy::must_use_candidate -D clippy::return_self_not_must_use` | passed |
| `cargo test --workspace --locked --no-fail-fast` | passed |
| `cargo test -p rake-native --features native-tests --test appkit --locked` | passed |
| `cargo test -p rake --features view-test-support --lib --test view_state --locked` | passed |
| `cargo test -p rake --features view-test-support,devtools --lib --test view_state --test element_inspection --locked` | passed |
| `cargo clippy -p rake --all-targets --features view-test-support,devtools --locked -- -D warnings` | passed |
| `cargo check -p renamed-facade --features devtools --locked` | passed |
| `cargo test -p rake-view --features view-test-support --lib --locked` | passed |
| `cargo check -p rake --no-default-features --locked` | passed |
| `cargo check -p rake --features native-components --lib --target wasm32-unknown-unknown --locked` | passed |
| `cargo check -p rake-diagnostics -p rake-protocol -p rake-http --lib --target wasm32-unknown-unknown --locked` | passed |
| `cargo check -p rake-macros --no-default-features --locked` | passed |
| `cargo test -p rake-client -p rake-http -p rake-protocol --locked` | passed |
| `cargo test -p rake-ui --features test-support --lib --locked` | passed |
| `cargo clippy -p rake --no-default-features --locked -- -D warnings` | passed |
| `cargo build -p rake-cli --locked` | passed |
| `python3 scripts/test-bundle.py` | passed |
| `python3 scripts/test-dev-bridge.py` | passed |
| `python3 scripts/test-supervisor.py` | passed |
| `python3 scripts/test-inspector.py` | passed |
| `node scripts/test-web-loading.mjs` | passed |
| `node scripts/test-inspector.mjs` | passed |
| `node scripts/test-inspector-ui.mjs` | passed |
| `node scripts/test-notes-storage.mjs` | passed |
| `node scripts/test-platform.mjs` | passed |
| `node scripts/test-web-runtime.mjs` | passed |
| `node scripts/test-offline.mjs` | passed |
| `node scripts/test-file-processing.mjs` | passed |
| `cargo test -p rake --features view-test-support --test actions --locked` | passed |
| `cargo test -p rake-files --lib --locked` | passed |
| `cargo clippy -p rake-files --lib --target wasm32-unknown-unknown --locked -- -D warnings` | passed |
| `python3 scripts/test-web-deployment.py` | passed |
| `cargo test -p rake --no-default-features --test platform --locked` | passed |
| `cargo test -p rake --test platform --locked` | passed |
| `cargo clippy -p rake-web-host-fixture --lib --target wasm32-unknown-unknown --locked -- -D warnings` | passed |
| `cargo clippy -p rake --no-default-features --target wasm32-unknown-unknown --locked -- -D warnings` | passed |
| `cargo test -p rake-inspector --no-default-features --locked` | passed |
| `cargo clippy -p rake-inspector --no-default-features --all-targets --locked -- -D warnings` | passed |
| `cargo test -p rake-notes --no-default-features --locked` | passed |
| `cargo clippy -p rake-notes --no-default-features --all-targets --locked -- -D warnings` | passed |
| `cargo test -p rake --no-default-features --test managed_work --test routing --test windows --test runtime_control --test reload --test web_deployment --test element_inspection --test application_commands --test guarded_quit --test services --locked` | passed |
| `cargo check -p rake-foundation -p rake-routing -p rake-view -p rake-inspector -p renamed-facade --lib --target wasm32-unknown-unknown --locked` | passed |
| `cargo check -p rake-view -p renamed-facade --lib --features rake/devtools --target wasm32-unknown-unknown --locked` | passed |
| `cargo clippy -p rake -p rake-foundation -p rake-routing -p rake-notes -p rake-view -p rake-inspector -p renamed-facade --lib --target wasm32-unknown-unknown --locked -- -D warnings` | passed |
| `cargo build -p rake-foundation -p rake-routing -p rake-notes -p rake-view -p rake-inspector --lib --target wasm32-unknown-unknown --locked` | passed |
| `cargo check -p rake-routing --lib --features hash-routing --target wasm32-unknown-unknown --locked` | passed |
| `cargo doc --workspace --no-deps --locked` | passed |
| `python3 scripts/check-spacing.py` | passed |

The workspace documentation command also ran with `RUSTDOCFLAGS=-D warnings`.

## Wasm measurement

Same machine, Rust 1.98.0, default dev profile, `rake-view`, clean separate
`CARGO_TARGET_DIR`, populated download cache, no concurrent Cargo work.
Command: `cargo build -p rake-view --lib --target wasm32-unknown-unknown --locked`,
then wasm-bindgen 0.2.129 `--target web`. Sizes are uncompressed bytes, including
dev debug information; they are not production transfer sizes.

| Stage | Clean Cargo build | Bindgen | Raw Wasm | Processed Wasm | JS |
| --- | ---: | ---: | ---: | ---: | ---: |
| Before Base | 77.80 s | 4.29 s | 552,094,257 | 109,064,684 | 190,792 |
| After Base | 99.57 s | 4.67 s | 631,499,827 | 110,892,873 | 190,792 |

Base added 21.77 s (28.0%) to this one clean build and 1,828,189 bytes
(1.68%) to processed dev Wasm. Raw debug Wasm grew 79,405,570 bytes
(14.38%). This is a single-run comparison, not a statistical benchmark.

## Phase 2

Base 0.9.0 builds on stable Web. It is reachable through `rake-ui`, which Rake
only compiles with `experimental-view`. Base Root wraps initial and additional
routed windows. Rake keeps its downcastable Button wrapper, forwards native
Element hooks directly to Base's rendered button, and retains authored and
layout-time resolved styles for inspection and temporary editing. A small
button-only state erasure preserves that direct Element boundary; no backend
abstraction was introduced. Inferred button style callbacks now receive Rake's
Button wrapper, including ParentElement, rather than a Stateful<Div>.

Base Dialog hosts modal surfaces, Base focus traps register modal scopes,
Base PopoverState supplies the lifecycle/deferred-context registration, and
Base Positioner places separately mounted Rake anchors. Rake retains controlled
stack ordering, sentinel traversal, opener-removal checks and frame reconciliation
to preserve its public overlay API and consumed-key behavior. Its dialog host
turns off Base's action key context so a child's consumed Escape takes priority.
Kit presentation uses this same Base Root rather than nesting another root.

The real AppKit test now mounts both windows under Base Root and tests window
accessibility hit testing. It failed before the fix (AccessKitNode over a native
button). Rake now intercepts the actual KVO window class, routes hits inside
registered native hosts through AppKit's native representation, and otherwise
forwards to Base's original method. The passing test compares against AppKit's
own button hit result (an NSButtonCell for this style). Existing native action,
clipping, accessibility, remount and teardown assertions remain in place.
This independently touches the same runtime-class issue as the cleanup branch;
that branch was not merged.

The Base Web view example was exercised with actual pointer/keyboard input:
increment, open popover, open nested document dialog, select row 5,000 in its
10,000-item virtual list, Tab/Shift-Tab, Escape one layer at a time, and theme
switch with count retained. No browser warnings/errors were captured. The
headless view and inspection suites also pass; a separate Base Root regression
covers traversal and consumed Escape under the actual application root.

## Phase 3

`rake::theme::Semantic` exposes Base's 18 semantic color roles as flat fields,
plus Base's appearance. Arbitrary existing theme structs still use the same
independent ambient/global storage. Non-color Base scales remain independent.
With Kit off, `install` projects the colors into Base and updates Rake's copy.
A real GPUI effect-queue test changes the Base global after installation and
checks that Rake's observed copy changes once; an unchanged Base notification
must produce no ambient replacement. The test also retains an unrelated app
theme to check that the bridge does not overwrite arbitrary theme structs.
