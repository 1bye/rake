# Landing the native-review snapshot

Source: `codex/rake-native-review` at `1cb59611aff2d79446ddb24c2e05784a73d59195`,
plus `/tmp/land-snap/uncommitted.patch` and `untracked.tar`. The source was
reconstructed independently under `/tmp/land-snapshot-state`; the live checkout
was only read, fetched, and used to register this worktree.

Destination base: `origin/main` at `8d0e365`, including #43 (`4325d77`), #44
and #45. The initial fetch was `c3ad00a`; #45 merged while verification ran.
All landing commits were still unpushed, so they were replayed onto the updated
main before final verification.
One draft PR on `codex/land-native-review`; no merge or push to main. Refs #42.

## Committed source work

- `5c818c4` -> `d672cd4`: examples roadmap, retained verbatim.
- `1cb5961`: initially cherry-picked and conflict-resolved as `4571b3c`, then
  **dropped from the final branch as superseded by #45 (`7974fcc`, merge
  `8d0e365`)**. Main now supplies the banner, Vermilion default icon, native
  icon generation, development/deployment favicon and their tests. Preserve
  #45's package-local `crates/rake-cli/scripts/default-icon.svg` and extraction
  path; discard the interim cross-crate fallback path. No branding changes are
  duplicated in this PR.

## Snapshot disposition and path map

| Snapshot input | Landing / disposition |
| --- | --- |
| `.github/workflows/check.yml` | Keep current CI/cache/toolchain structure; exclude retired inspector from build matrices; replace inspector suite with runtime-broker and embedded-devtools suites. |
| `Cargo.toml`, `Cargo.lock` | Remove only the inspector member/package; add its workspace exclusion; preserve current versions, vendor patches, members and exclusions. |
| `crates/rake-cli/src/cdp/mod.rs` | Explicit `--elements` deprecation. |
| `crates/rake-cli/src/main.rs` | Route `inspect` to CDP after the existing `inspect pick` terminal path. Preserve all current native doctor/build and distribution dispatch. |
| `crates/rake-cli/src/project.rs` | Embed `runtime_broker.py`, `embedded-devtools.mjs` and `.html` under `../scripts/`; stop embedding inspector.py. Keep current preview, fonts and other helpers. |
| `scripts/dev.py` | `crates/rake-cli/scripts/dev.py`: broker import, owned Node companion after Web publication, Node 22 prerequisite, private toggle URL, deprecated inspector rejection. Keep current dependency naming, canonical paths and desktop-without-preview server bypass. |
| `scripts/serve-web.py` | `crates/rake-cli/scripts/serve-web.py`: runtime broker, control-only mode with old flag alias, CDP discovery, error-only status overlay and toggle button, HTTP 410 for retired frontend routes. Keep current repository root calculation and request boundaries. |
| `scripts/inspector.py` (deleted) | Delete `crates/rake-cli/scripts/inspector.py`; retain its forwarding/security behavior in runtime_broker.py. Main had already removed the HTML/CSS/JS frontend files. |
| `scripts/build-web.py` | Repository-only helper stays in scripts/; remove retired example; retain #45's relocated favicon import. |
| `scripts/test-bundle.py` | Retain main's native packaging tests; add production archive exclusion assertion and honor CARGO_TARGET_DIR. |
| `scripts/test-inspector.py` (deleted) | Replace with test-runtime-broker.py, preserving forwarding, source confinement, discovery and bounded output coverage, plus CDP attach and deprecation coverage. |
| `scripts/test-preview-native.py` | RuntimeBroker import; honor CARGO_TARGET_DIR. |
| `scripts/test-supervisor.py` | Keep current suites; add owned companion/republication/shutdown test; honor CARGO_TARGET_DIR. |
| `docs/development.md`, `docs/devtools.md` | Document current desktop launch behavior, Web embedded DevTools, Node 22, private URLs, lazy frontend download and inspect/CDP alias. Use current crate names and helper paths. |
| `docs/devtools-next.md` | Update relocated `docs/internal/devtools-coverage.md`; do not resurrect the old path. |
| `docs/rake-inspector.md` | Update relocated `docs/internal/legacy-inspectors.md`; replace obsolete launch instructions with deprecation and mark remaining details historical. |
| `docs/element-inspection.md` | Add embedded boundary suite invocation, keep current reference. |
| `docs/experimental-view.md` | Superseded: main's redirect to docs/view.md is retained. The old patch would resurrect an obsolete reference. Remove retired inspector from current docs/view.md verification commands instead. |
| `docs/guarded-quit.md` | Superseded: main already removed inspector references and points to maintained test coverage. Keep current text. |
| `docs/notes.md` | Embedded Web DevTools command and authored hierarchy, adapted to current shared editor controls and privacy boundary. |
| `examples/notes/Cargo.toml` | Superseded: experimental-view is now an empty compatibility feature. Keep current gui/gpui-base and view-test-support wiring. |
| `examples/notes/src/editor.rs` | Superseded: main deleted the custom Editor in favor of shared Input/Textarea. Do not restore it. |
| `examples/notes/src/view.rs` | Port the authored view! shell, landmarks, keyed rows and mounts around main's shared inputs. Keep accessibility sync, keyboard scope, focus handles, validators, edit-limit messages, persistence and selection code. |

New snapshot files:

- `scripts/runtime_broker.py`, `scripts/embedded-devtools.{mjs,html}` move to
  `crates/rake-cli/scripts/` and are embedded by project.rs.
- `scripts/test-runtime-broker.py` and `scripts/test-embedded-devtools.mjs` stay
  repository-only, with relocated imports/launch paths.
- `scripts/test-notes-view.py` updates main's existing live-capture test: authored
  hierarchy and keyed rows plus current focus/privacy assertions. Shared controls
  are opaque GPUI children, rather than the removed custom Editor `<view>` mount.
- Include all three supplied `.agents/plans/` and both `.agents/reports/` files,
  `docs/future/full-stack-web.md`, and `examples/inspector/README.md`.
- Exclude all 14 `.agents/issues/` drafts. Their README says GitHub is authoritative
  and the drafts are not kept in sync; they are unrelated planning input here.
- Exclude generated `output/` entirely, along with macOS tar metadata (`._*`).

## #43 and remaining work

Ordinary doctor/build remain native Rust under `src/project_build/`, including
canonical manifest matching, diagnostics, first-lock handling, staging/rollback
and process cleanup. Existing native distribution dispatch also stays intact.
Main intentionally bypasses the Web control server for desktop dev without
preview; retain that behavior rather than reinstating a legacy inspector origin.

#45's Python `install_favicon` behavior is active in Web dev,
Web bundles and the repository build-web helper. **Follow-up:** port this default
favicon insertion/preservation/collision behavior to the #43 Rust Web builder in
`src/project_build/web.rs`. Its one-shot builds keep #43's existing HTML behavior;
it is not routed back through Python. Add equivalent Rust integration coverage
when doing that port. The current native doctor/build help also retains its
phase-1 compatibility flag descriptions; refreshing these is a documentation
follow-up, not a reason to restore inspector dispatch.

For #42 phase 2, port the supervisor that is now here: file watching and
fingerprinting, rebuild/publication, launch/restart and reload coordination,
process-group shutdown, private session/status/output records, CDP ownership and
the Web-only Node companion lifecycle. Python runtime broker, serving, bundling
and native preview remain later phases; Chromium frontend assets stay lazy and
the pinned Node frontend/proxy remains a separate owned process.

## Verification

All build/test commands use `CARGO_TARGET_DIR=/tmp/land-review-target`,
`CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0`. Rust is the existing
1.98.0 toolchain; Node is 26.10.0. No toolchains/packages installed.

Final verification results:

| Check | Result |
| --- | --- |
| `cargo test -p rake-app-cli --no-fail-fast` | Passed on the final base: 270 passed (192 bin unit + 78 integration), 2 pre-existing ignored, 0 failures. |
| `cargo clippy -p rake-app-cli -p rake-build -p rake-notes --all-targets --features rake/devtools --locked -- -D warnings` | Passed on the final base. |
| `cargo fmt --all --check`; `rake format --check examples/notes/src/view.rs` | Passed. |
| `rake lint` | Passed on the final base: 699 Rust files, 14 diagnostics, 0 denied. |
| `cargo test -p rake-build -p rake-notes --features rake/devtools,rake-notes/view-test-support --locked` | Passed: rake-build 1 unit + 2 icon tests; notes 12 unit + 15 persistence tests; empty doc/bin targets also pass. |
| `cargo check -p rake-notes --features rake/devtools` | Passed. |
| `cargo clippy -p rake-notes --lib --features rake/devtools --target wasm32-unknown-unknown --locked -- -D warnings` | Passed on the final base. |
| `python3 scripts/test-supervisor.py` | 14 passed, no skips. |
| `python3 scripts/test-runtime-broker.py` | 9 passed. |
| `python3 scripts/test-dev-bridge.py` | 4 passed. |
| `python3 scripts/test-preview.py` | 18 passed. |
| `python3 scripts/test-web-deployment.py` | 10 passed. |
| `node --test scripts/test-embedded-devtools.mjs` | 1 passed. Uses cached frontend doubles; does not establish real Chromium UI behavior. |
| `python3 scripts/test-bundle.py` | 6 passed, 1 failed: missing matching wasm-bindgen 0.2.129. No installation attempted. |
| `python3 scripts/test-preview-native.py` | Mixed: initial real native run passed; final-base repeat failed with `Inspection: StaleGeneration` during the blur-toggle click; standalone recheck passed. Assertions/test logic were not changed. |
| `python3 scripts/test-notes-view.py --session PRIVATE_SESSION` and `--snapshot /tmp/land-review-notes-capture.json` | Both passed against a real visible native notes app: 100 captured nodes. Snapshot recheck on the final base also passed. The app used a private temporary data directory and was stopped afterward. |

Additional attempts and limits:

- The first CLI integration attempt selected system Python 3.9 and failed the
  distribution extension fixture at `hashlib.file_digest`. Using the already
  installed Python 3.14.4 first on PATH fixed it; the complete rerun passed.
- The snapshot's own reconstructed `scripts/dev.py bundle --target web` path,
  using its own bundle fixture/lock version, also exits 1 with exactly the same
  missing wasm-bindgen 0.2.129 error. This confirms the bundle environment gap
  predates the port. The entire old snapshot suite was not claimed as green.
- An extra Wasm `--all-targets` clippy attempt failed with E0425 in the unchanged
  `examples/notes/src/main.rs`: the native binary calls `rake_notes::main`, which
  the app macro gates out on Wasm. The supported `--lib` Wasm command passes.
  Both the binary entry point and lib app annotation match main unchanged.
- No tests were weakened, deleted to hide failures, or silently skipped. The
  inspector suite was replaced as part of the requested frontend retirement;
  runtime/security coverage remains in test-runtime-broker.py.
- No live Web/Chromium frontend or Windows run. Native preview has an intermittent
  generation race to investigate; no claim of stable GUI behavior across repeats.
- Keep `/tmp/land-review` for fixes; remove `/tmp/land-review-target` after checks.
  Logs and the notes capture are retained under `/tmp/land-review-*.log` and
  `/tmp/land-review-notes-capture.json` outside the commit.

No user decisions were needed. Open follow-ups: native Rust Web default favicon,
phase-1 help cleanup, intermittent native preview generation race, and #42 phase 2.
