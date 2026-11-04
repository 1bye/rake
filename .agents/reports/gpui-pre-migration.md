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
and runs on stable. Browser background work runs cooperatively on the main thread, matching
Rake’s previous CE single-threaded constructor. CPU-heavy work can delay input/rendering; no equivalent CPU stress
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
browser also reported `crossOriginIsolated === false` and `SharedArrayBuffer`
unavailable, confirming no cross-origin isolation requirement for this path. The
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

## Phase 4

The optional native `kit` feature reexports Kit through `rake::kit`, initializes
its presentation plugin/assets and uses the same Base Root. Default Rake and
ordinary workspace builds do not enable Kit. The pin check passed for GPUI
0.3.11 and Base/Kit/Component/Assets 0.9.0 and verified default and stable Web
dependency graphs.

Kit's `Theme::update` and direct mode-change paths passed a real Base observer
test. Semantic installation switches Kit's mode before applying colors and
uses Kit's canonical write path; it explicitly preserves `selection` because
Kit 0.9's semantic setter omits that role. The observer reads Kit's
`semantic_tokens()` and skips unchanged values. Existing app themes are retained.

Kit owns the window-wide dialog/sheet stack and notifications. Local Rake
overlays suspend rendering and focus guards during global modals and resume
afterwards. A headless regression opens Kit over a retained Rake dialog and
checks that focus yields and returns. All 31 view-state cases passed with Kit.

The window inspection decorator captures application and Kit layers through the
end of GPUI deferred paint. Its regression first failed because a global dialog
boundary was absent; it now checks that dialog content was painted, Rake content
remains editable, and boundary identities survive rerender. Kit's private
component descendants and internal resolved styles remain opaque; explicit
boundaries expose geometry, authored wrapper styles, source/handler declarations,
and caller-supplied focus handles. No full-tree parity is claimed.

An isolated registry-only probe with `gpui-kit = { version = "=0.9.0",
default-features = false }` failed on stable 1.98 in `wasm_thread` 0.3.3 with
E0554 (`stdarch_wasm_atomic_wait`). Rake therefore gates Kit to native targets
and emits a deliberate error on Wasm. Core/Base stable Web remains supported.
The [Kit example](../../examples/kit/README.md) is native-only; it uses routed tabs,
Kit button/dialog/table boundaries and one Semantic palette. No upstream source
was patched, forked or vendored, and GPUI Fast was not added.

## Native Kit verification

Ran the bundled, ad-hoc signed `Rake Kit.app` on macOS 26.6.2 arm64. Actual
pointer input incremented the Kit button, opened its global dialog, and switched
the shared Semantic theme from that dialog. The page, button, dialog and table
changed together. Native accessibility exposed the table/row/cell roles.

A separate fresh run opened the components route in a new tab through Rake's
control API, incremented with the pointer, selected Home and then Components
through the control API, and confirmed the presented count remained 1. This
checks retained route state; the tab changes were API-driven, not pointer-driven.
The live inspection tree included `kit.increment`, `kit.dialog.trigger`,
`kit.table` and the global `kit.dialog.content` boundary. Kit boundaries were
opaque/non-editable, with explicit button focus navigation; the Rake-authored
button inside the Kit dialog remained style-editable and had deferred paint
order. Test applications were quit through their own control endpoint.

## Additional compatibility findings

GPUI pre's macros rewrite generated paths to any Kit dependency declared in a
manifest, even if that optional dependency is disabled. Rake UI uses explicit
`IntoElement` implementations for its four overlay views and ordinary GPUI test
contexts for its semantic tests, avoiding an upstream source patch or an
always-enabled Kit dependency. The disabled-Kit native and stable Web gates
exercise this configuration.

A new button role assertion first failed with `Button` instead of the authored
`MenuItem`; the Base wrapper now forwards the authored role. The existing hover
and live-edit assertions remain. The outside-click overlay test now covers left,
right and middle buttons with the same dismissal/background assertions. A small
backdrop handler preserves Rake's all-button dismissal around Base's left-button
handler. No existing assertion was weakened for either change.

## Final migration gates and decisions

The complete command table above was also exercised after Kit integration.
Initial optional-Kit derive failures were fixed and all affected gates rerun.
Final workspace tests used `--no-fail-fast`; AppKit ran against real macOS native
controls under Base Root. Documentation passed with warnings denied. The pin
check also rejected a deliberately mismatched Kit pin in a temporary scratch
manifest; the real manifest was not changed for that negative check.

| Additional command | Result |
| --- | --- |
| `python3 scripts/check-gpui-pins.py` | passed |
| `cargo test -p rake-ui --test semantic --locked` | passed |
| `cargo test -p rake-ui --features kit --test semantic --locked` | passed |
| `cargo test -p rake --features kit,view-test-support,devtools --lib --test view_state --locked` | passed |
| `cargo clippy -p rake --features kit,view-test-support,devtools --all-targets --locked -- -D warnings` | passed |
| `cargo clippy -p rake-kit-example --features kit,devtools --all-targets --locked -- -D warnings` | passed |
| `cargo doc -p rake --features kit --no-deps --locked` with `RUSTDOCFLAGS=-D warnings` | passed |
| `cargo check -p rake --features kit --lib --target wasm32-unknown-unknown --locked` | expected rejection: Rake's native-only diagnostic; no upstream E0554 |
| `python3 scripts/build-web.py --example view` | passed; final bundle also exercised in Chromium |
| `target/debug/rake bundle --target desktop --package rake-kit-example --features kit,devtools --debug --json` | passed; bundled app run manually |

One later parallel Kit run also hit the recorder integration test's final
empty-page assertion. Its cursor covered only the first page of the stopped
buffer. GPUI's process-wide event buffer and per-App window IDs allow concurrent
test applications' draws to fill additional pages. The test now drains the
stopped buffer before drawing again; its assertions that recording is off and
that the later draw adds no frames are unchanged. Production frame collection
is unchanged. Both affected non-Kit and Kit library/input suites passed afterwards.
This is separate from the known CDP lost-submission flake.

The final rebuilt browser bundle passed increment, nested popover/dialog,
right-click backdrop dismissal of only the top dialog, Escape and theme switching
with count retained. No warnings/errors were captured. The original notes
storage/recovery coverage remains as described above. Inspection highlights
are painted after Kit's deferred layers; the dialog regression also checks a
highlight is laid out for the captured boundary.

The owner's follow-up authorized investigating disabled multithreading. That
resolved the stable core/Base Web decision and migration continued through all
four phases. Kit Web remains gated as allowed by Phase 4; no nightly workaround
was added. Follow-ups are upstream control of Kit's Web threading feature,
Windows/Linux and Safari/Firefox runs, suitable fonts for broader glyph coverage,
a CPU-bound browser latency benchmark, and renderer-crash recovery through an
allowed browser harness. These are unverified, not promised behavior.

## Implementation commits

The final documentation commit follows these commits. No push, merge, tag or PR
was made; `main` remained at `5fc0f9d` when the work was finalized.

- `e27935c` build: pin GPUI pre and Rust 1.98
- `73bfff0` fix(gpui): adapt views and inspection to pre
- `6477ead` fix(http): bridge GPUI pre request transport
- `448bfdf` fix(native): detach views when AppKit closes
- `1bf6434` fix(web): restore bundled default fonts
- `19d2729` style: satisfy Rust 1.98 and spacing lints
- `42ab433` docs: record GPUI pre stable Web verification
- `bc5221c` feat(view): build Rake controls on GPUI Base
- `e26f357` fix(native): compose Base accessibility hit tests
- `9b7eaa9` docs: record Base verification and Wasm costs
- `799dcbf` feat(theme): synchronize flat semantic colors
- `d7324ca` fix(view): retain authored button metadata
- `a00ea96` feat(kit): add optional native Kit integration
- `4624da4` fix(inspect): capture Kit presentation layers
- `7698586` test(inspect): drain stopped frame pages
- `0645471` ci: verify GPUI pins and optional Kit paths

## Migration review and fixes — 2026-11-04

Reviewed the implementation from `5fc0f9d` through the migration, treating Rake
as a Rust 2024 library on MSRV 1.98 with one GPUI application/UI thread and
multiple retained tabs/windows. Emphasis was overlay ordering, focus ownership,
canonical theme writes, and the deferred inspection lifecycle. Local published
GPUI/Base/Kit sources supplied the upstream contracts. Rules below refer to the
project's [Rust workflow](../../.agents/workflow/rust/AGENTS.md),
[architecture guide](../../.agents/workflow/rust/architecture.md), and
[testing guide](../../.agents/workflow/rust/testing.md). No cleanup branch was merged.

### Must fix — completed

1. **[P2] Disabled buttons retained their old tab stop.**
   Location: `crates/rake-ui/src/view/button.rs`, `Button::request_layout`.
   Base skips its focus configuration for an unfocused disabled button. A button
   previously rendered as enabled therefore remained reachable by Tab after being
   disabled elsewhere. Rake now updates its retained handle's tab index/stop before
   passing it to Base. The new transition test failed before the fix; the browser
   also verified that Shift-Tab skips newly disabled Increment and Reset controls.
   Rule: preserve observable public behavior and test transitions (project Rust
   workflow/testing guide); make state/resource ownership explicit (Google Rust
   Style Guide ownership guidance).
   Fix: `.tab_index(self.tabindex).tab_stop(!self.disabled && self.tabindex >= 0)`.

2. **[P2] Local overlay mutations stole Kit modal focus.**
   Location: `crates/rake-ui/src/overlay/state.rs`, `OverlayState::open/close` and
   `OverlayHostState::audit`; popup registration in `overlay/elements.rs`.
   Rendering/audit suspension alone did not cover programmatic opens and closes:
   `close` restored a local opener while Kit was active, and `open` focused an
   unmounted local surface. The failing regression now passes for closing and
   reopening during Kit presentation, including subsequent frame audits and
   restoration after Kit closes. Restoration uses a weak pending target and a
   mounted fallback if that target disappears. Hidden popovers release Base's
   deferred interaction context and reacquire it on resume, tested independently.
   Rule: one concrete owner for state/resources, no redundant competing ownership
   (project architecture guide; Google Rust Style Guide ownership guidance).
   Fix: defer local focus work while `kit_modal_open`; use Base's controlled
   `PopoverState::set_open` for registration, leaving focus to Rake's controller.

3. **[P2] Window highlights used the previous frame's geometry.**
   Location: `crates/rake/src/inspection/view/window.rs`, `EndCapture::prepaint`.
   The window wrapper built its highlight before deferred content was fully
   captured and identified. A moving target at x=90 still highlighted x=10.
   Identity assignment and highlight preparation now happen at the final deferred
   prepaint sentinel using the current capture; publication and highlight paint
   remain after deferred paint. The regression failed before the fix and also
   checks that removing the target leaves no ghost highlight. The test changes
   geometry without a separate entity notification, preventing an extra draw from
   concealing a stale first frame.
   Rule: derived snapshots need a defined refresh boundary (project architecture
   guide, ownership/derived-state section).
   Fix: identify the final prepaint capture, then call `overlay` with that snapshot.

4. **[P2] Rake initialization reset an existing Kit host's theme.**
   Location: `crates/rake-ui/src/lib.rs`, `init`.
   Kit initialization resets its theme/registry. Calling it again when adding
   Rake controls to an initialized Kit application changed dark appearance and
   custom colors. Rake now adopts an existing Kit theme global and only performs
   Kit initialization when absent. Its own marker still installs one Rake theme
   observer. The regression failed before the fix and now checks appearance,
   primary color and radius, including after GPUI's queued effects run.
   Rule: preserve caller-owned state during library initialization (project Rust
   workflow, API/ownership guidance).
   Fix: guard `gpui_kit::init` with `!cx.has_global::<KitTheme>()`.

### Complexity removed and design retained

The floating renderer is now only an anchored popover path. Removed its unused
optional dialog title/footer, the redundant forwarding function, and the
unreachable centered-dialog positioning branch. Passing the required anchor
explicitly removes optional-state branches. Rake's focus controller no longer
starts Base's second focus/restoration lifecycle for its popup entity.

The button's narrow rendered-element adapter remains justified: Base hides its
rendered type, and forwarding native Element hooks preserves accessibility and
Rake's inspectable boundary without adding a second GPUI node. The real
Base-global observer and equality guard remain unchanged; Base/Kit theme tests
still verify actual notifications, independent app themes and no notification loop.
Exact ecosystem pins, one shared Root and stable Web gating also remain intact.

No existing assertion was weakened, removed or ignored for these fixes. The
existing ignored `publish_benchmark` is a manual benchmark, not a skipped
correctness regression. The new tests are included by existing CI commands.

### Review verification

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | passed |
| `cargo check --workspace --all-targets --locked` | passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | passed |
| `cargo clippy -p rake --features kit,view-test-support,devtools --all-targets --locked -- -D warnings` | passed |
| `cargo test -p rake --features view-test-support,devtools --lib --test view_state --test element_inspection --locked` | passed |
| `cargo test -p rake --features kit,view-test-support,devtools --lib --test view_state --locked` | passed |
| `cargo test -p rake-ui --test semantic --locked` | passed |
| `cargo test -p rake-ui --features kit --test semantic --locked` | passed |
| `cargo check -p rake-view -p renamed-facade --lib --features rake/devtools --target wasm32-unknown-unknown --locked` | passed |
| `cargo doc -p rake --features kit --no-deps --locked` | passed |
| `python3 scripts/check-gpui-pins.py` | passed |
| `python3 scripts/check-spacing.py` | passed |
| `python3 scripts/build-web.py --example view` | passed |

Kit library suite: 120 passed, one existing manual benchmark ignored; Kit input
suite: 34 passed. Without Kit: 118 library, 31 input and two inspection integration
tests passed. Both semantic paths passed. Rustdoc ran with warnings denied.

The rebuilt stable Web example also ran in Chromium with actual pointer/keyboard
input: increment; anchored popup; nested dialog; virtual-list jump to 5,000;
Tab/Shift-Tab; Escape closes one layer and visibly restores each opener. A lock
shortcut disabled Increment/Reset while they were unfocused, and two backward
Tab steps skipped them to the retained history row. No console warnings/errors
were captured. The disposable server/tab were closed afterward.

Not rerun in this review: the whole workspace test suite, AppKit hardware suite,
notes storage matrix, Windows/Linux, Safari/Firefox, renderer-crash recovery or
performance benchmarks. Earlier migration results retain their stated scope.
Kit modal/theme/inspection fixes were exercised through GPUI's real global,
layout, input and focus machinery with TestPlatform; no new live Kit GUI run is
claimed. Broader glyph coverage remains the documented font limitation.

Verdict: all four correctness findings fixed and verified in the selected scope.
No remaining actionable findings from this review. Nothing merged or pushed.

Review fix commits (the documentation commit follows):

- `c28bc43` — clear disabled button tab stops.
- `14f44ea` — preserve Kit modal focus ownership and remove dead popup branches.
- `38f87e4` — adopt existing Kit initialization.
- `dcaea12` — highlight current deferred geometry.

## Full-suite verification and merge readiness — 2026-11-04

Tested review-fix commit `ed0a67e8e03671f61f22022266711f708f30d16d` in
`/private/tmp/rake-gpui-pre-kit-base`, branch `codex/gpui-pre-kit-base`, on
macOS arm64 with the pinned Rust 1.98.0 toolchain. The worktree was clean before
verification; `main` remained at `5fc0f9d`. This follow-up closes the workspace
and AppKit coverage gap noted in the review above.

| Command | Result | Elapsed |
| --- | --- | ---: |
| `cargo test --workspace --locked --no-fail-fast` | passed | 257.04 s |
| `cargo test -p rake-native --features native-tests --test appkit --locked` | passed | 4.69 s |

The full workspace run included unit, integration, consumer/compile fixtures
and doctests: 668 passed, zero failed, two existing manual performance benchmarks
ignored (`cdp_projection_baseline` and `cdp_refresh_load_benchmark`). The known
lost-submission CDP test passed on its first run. No retries or source fixes were
needed. This is the normal workspace feature selection; optional Kit behavior
is covered by the separate passing feature suites recorded in the review.

The AppKit executable opened real native windows under Base Root and completed
its actions, symbols, opacity, clipping, accessibility/hit-testing, remount,
window-move, close and teardown checks. Its parent process verified the child's
completion marker as well as its successful exit status. It was not skipped or
substituted with TestPlatform.

**Merge recommendation: ready for the agreed migration scope.** The reviewed
code has no known blocking findings, the requested full suites now pass, and
the earlier targeted Kit, lint, documentation and stable-browser verification
remains applicable to the same source. The subsequent change only records these
results. Nothing has been merged or pushed.

Scope limits remain: Kit 0.9.0 is intentionally native-only; core/Base Web uses
the verified stable single-threaded path. Windows/Linux, Safari/Firefox,
renderer-crash recovery and the previously noted broader glyph coverage are
not newly verified by these suites. This is a merge assessment, not a claim of
release validation on every platform.
