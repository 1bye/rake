# Project toolchain port

## Phase 2: native desktop supervisor slice

Refs #42. Desktop `rake dev` without `--preview` now runs in Rust, including
`--cdp`, `--native-reloader`, `--restart-without-state`, and
`rake dev status --session PATH`. This is a complete desktop slice, rather than
claiming the whole phase-2 supervisor migration is finished.

`main.rs` calls `project_build::native_dev` before the embedded adapter. It uses
the same option inventory as the parser, recognizes abbreviated/attached flags,
and respects the final target selection. **Every Web or preview-enabled dev
session still executes the entire unchanged Python supervisor.** Its embedded
helper extraction, arguments, runtime broker, Web server and Node companion
lifecycle remain together. Native desktop sessions neither extract nor run Python.
Default status reads can also inspect retained Web/preview session records.

### Implementation mapping

| Python contract | Rust owner in `src/project_build/` |
| --- | --- |
| `Project.__init__`, local watch roots and `build_features` | Shared `Project::load`, `Project::build_features`, existing native build/staging adapter |
| `Project.fingerprint` | `dev/fingerprint.rs`: SHA-256 contents, sorted walks, excluded outputs/directories, no symlink traversal, overlapping-root deduplication |
| `Supervisor.run` | `dev/mod.rs`: debounce, owned build worker, metadata refresh, discard superseded/failed builds, numbered publication, busy retry, crash observation |
| `Supervisor.launch`, `publish`, `exchange` | `dev/mod.rs`, `dev/transport.rs`: app groups, same keyed retry, quiescence/checkpoint, restart, replacement backend, abort and restore outcomes |
| `private_json`, session/status branch | Existing atomic `write_marker`, `dev::run`: private `/tmp/rake-dev-*`, control/build directories, status/output/checkpoint/CDP files |
| `ProcessOutput`, pipe collectors | `dev/output.rs`: 128 records, byte truncation, character-bounded UTF-8 lines, structured prefix, lifecycle records, final flush |
| Owned build/app/CDP/reloader groups and signal shutdown | Shared `Processes`/`ProcessGroup`, cooperative cancellation, joined worker/readers, advertisement cleanup |
| Native `start_server` CDP branch | Owned CDP CLI child, cancellable bounded readiness, private `cdp.json`; desktop without preview keeps its server bypass |
| `bundle.py: install_favicon` | `favicon.rs`, called by Rust `web.rs`: shared icon, real HTML tags/decoded attributes, source-preserving insertion, authored-icon preservation and collision rejection |

Session JSON fields and event names, output cursor/loss fields, numbered builds,
control filenames, reload environment variables and exit codes 0/1/2 are retained.
Busy publication retries the existing artifact; failed compilation keeps the live
app; a failed replacement backend sends the matching abort and preserves its host.
Signals also abort an outstanding reload preparation before final cleanup.
Successful and superseded builds remain isolated through the existing native
packaging/staging implementation. A metadata refresh that adds local watch roots
is fingerprinted again before publication.

Preserved quirks include dev-only feature auto-enablement, retained old successful
builds, no automatic relaunch after a crash until another source change, ignoring
`--session` when starting a fresh dev run, and retaining old status fields until
replacement discovery arrives. A successful app restore can consume/remove its
checkpoint file; the supervisor does not recreate it after consumption.

### Differences and compatibility limits

- Help/text errors identify `rake`; native dev help removes the Python prerequisite
  claim and uses the current CDP/deprecation descriptions. Invalid dev choices
  follow the installed Python 3.14 argparse wording. Build/doctor keep phase-1 help.
- As in phase 1, OS errors and malformed JSON/TOML use Rust diagnostics, integers
  are bounded by Rust types, and manifest selection keeps the canonical-path fix.
  JSON object key order is not guaranteed; field names, types and compact private
  records are preserved. Elapsed times, process IDs and random session/build
  scratch names naturally differ.
- Favicon parsing uses installed html5ever rather than Python HTMLParser. Valid
  application templates and the existing behavior cases match byte-for-byte;
  malformed HTML error recovery (for example duplicate attributes) follows HTML5.
- Long-lived children reuse phase 1's Unix process groups. The Unix CLI gate
  remains; this change makes no Windows support claim. No dependency versions or
  toolchains were installed: SHA-256 and HTML tokenization reuse locked crates.

### Test mapping

| Existing behavior | New Rust coverage |
| --- | --- |
| Supervisor watch/failure isolation/restart/crash/status | `project_dev::watches_publishes_keeps_old_app_on_failure_restarts_and_survives_crash` |
| Explicit stateless restart policy and owned CDP | `blocks_restart_without_opt_in_and_owns_cdp`; `cdp_startup_failure_keeps_exit_one_and_cancellable_readiness_stops_cleanly` |
| Owned descendants, SIGINT/SIGTERM, compiler cancellation | `cleanup_kills_descendants_that_ignore_sigterm`; `cancellation_during_build_cleans_compiler_group_and_stops_session` |
| Superseded artifacts and local dependency watch roots | `superseded_build_is_discarded_before_publication`; `metadata_refresh_includes_new_local_watch_roots_before_publication` |
| Same-key transport retry; external backend failure/apply | `dev::tests::native_transport_retry_keeps_the_same_request_identity`; `native_backend_failure_aborts_and_success_applies_without_stopping_old_host` |
| Busy decisions, checkpoint restart, preparation cancellation, restore outcomes | `stateful_restart_prepares_checkpoint_and_busy_operations_delay_publication`; `cancellation_during_preparation_aborts_and_preserves_the_old_app`; `restore_failure_is_reported_and_clears_after_runtime_recovery` |
| Fingerprints, feature/dispatch boundary, bounded private output | Remaining `dev::tests`, plus `dev::output::tests` |
| Default favicon/authored icons/comments/scripts/entities/implicit head/reuse/collision/rollback | `project_build::web_default_favicon_preserves_authored_links_and_rejects_asset_collisions` |

Existing script suites remain intact. The previous phase-1 exact HTML assertion
now includes the requested default link; it still checks the entire output.
The new supervisor tests use shell tool doubles and real sockets/process groups,
without Python, Node, a GPU, a wasm loader or network access.

### Verification: macOS, Rust 1.98.0

Python 3.14.4 first on PATH, Node 26.10.0. All Cargo work used
`CARGO_TARGET_DIR=/tmp/rake-42-phase2-target` and
`CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`.

- `cargo test -p rake-app-cli --no-fail-fast`: **288 passed, 2 existing ignored**
  (202 binary unit tests, 86 integration tests).
- `cargo clippy -p rake-app-cli --all-targets --locked -- -D warnings`: passed.
- `cargo fmt --all --check`, changed-file `rake format`, and `rake lint`: passed;
  lint checked 706 Rust files, with 14 existing diagnostics and 0 denied.
- Script suites: supervisor **14 passed**, dev bridge **4 passed**, runtime broker
  **9 passed**, preview unit **18 passed**, Web deployment **10 passed**, Node
  embedded-devtools **1 passed**.
- A repeat initially raced the fixture's first structured output; the lifecycle
  test now waits for that output before replacement and retains its final-output
  assertion. The focused suite passed after this readiness fix.
- Bundle: **6 passed, 1 failed**, because matching wasm-bindgen 0.2.129 is absent.
  This is the known pre-existing environment gap; nothing was installed.
- Native preview: first run hit the documented `Inspection: StaleGeneration`
  during blur toggling; an unchanged standalone rerun passed all assertions.
- Real notes window launched by the Rust supervisor with owned CDP: live and
  saved-capture `test-notes-view.py` passed (**100 nodes**). A watched rebuild
  restarted the real app, restored checkpoint generation 2 from the original
  runtime, passed the same live hierarchy test again, then stopped on SIGTERM
  with exit 0 and final stopped records. Readiness waits account for replacement
  discovery and checkpoint consumption by the runtime.
- Differential against unchanged Python: native compile-failure/restart/crash/
  SIGTERM lifecycle matched normalized events, status, output, modes and numbered
  build retention; **9 argument JSON/status cases** matched exactly, plus missing
  reloader exit-code parity; **13 favicon cases** matched output bytes and success/
  collision behavior. PIDs/session paths/elapsed times were normalized.

No Linux/devbox run, real Chromium frontend, real ABI-replacement backend, or
Windows verification. The reloader success/failure paths use an external backend
double. Disk stayed above 110 GiB free. The live checkout was only fetched and
used to register the requested worktree; it was never edited. Build targets and
ignored verification logs are removed after recording results; worktree retained.

### Remaining work

- Remaining phase 2: port Web and preview-enabled session supervision, including
  Web build publication/server readiness, Node 22 checks and the owned embedded
  DevTools companion. Until that complete slice lands, the explicit Python
  dispatch boundary above stays in place.
- Phase 3: Web serving/bundling/offline payloads remain Python.
- Phase 4: runtime broker and dev bridge remain Python; current Rust CDP/inspect
  entry points stay intact.
- Phase 5: preview orchestration/capture transport remains Python/Swift/assets.
- Phase 6: remove embedded Python and its prerequisite docs; address process
  portability and the Unix gate. Do not remove helpers while fallback paths use them.
- Separate existing follow-up: intermittent native preview generation race.
  No user design decisions are outstanding for this slice.

## Phase 1 implementation and verification (historical)

`doctor` and the legacy one-shot `build` adapter run in Rust. Existing native
distribution modes keep their dispatch. The embedded scripts continue to support
`dev`, Web `bundle`, preview and serving. The native-review landing routes `inspect` through the existing Rust
CDP adapter and replaces inspector.py with runtime_broker.py plus an owned
Web DevTools companion. See [.agents/reports/land-native-review.md](../../.agents/reports/land-native-review.md)
for the landing map and the native Web default-favicon follow-up.

## Implementation mapping

| Python in crates/rake-cli/scripts | Rust in src/project_build |
| --- | --- |
| dev.py: Parser, build/doctor argument validation | args.rs, help.txt |
| dev.py: doctor | mod.rs: doctor |
| dev.py: Project.__init__ (build inputs only) | mod.rs: Project::load |
| dev.py: Project.build_features (build only) | Args.features, forwarded unchanged |
| dev.py: Project.build (desktop) | mod.rs: Project::build, existing rake-dist compatibility adapter |
| dev.py: Project.build (Web) | web.rs: build |
| bundle.py: BundleConfig validation/input, copy_tree | web.rs: bundle_assets, input, copy_tree |
| dev.py: staged build publication, private_json | mod.rs: build, write_marker |
| dev.py: Processes, run_output | processes.rs: Processes, ProcessGroup |

Web compilation and input copying are separate from publication and CLI parsing
so later phases can reuse them without porting deployment or serving now.

## Behavior and boundaries

- Desktop defaults to debug and uses the existing installed native layout.
  Web uses locked Cargo builds, the selected cdylib, matching wasm-bindgen,
  static inputs, font licenses, and the same fallback index.
- Preserve first-lockfile resolution, explicit features without automatic
  devtools, output markers, overlap/link/collision rejection, staging/rollback,
  bounded metadata, rendered Cargo diagnostics, and exit codes 0/1/2.
- Preserve the diagnostic JSON field order, Python JSON spacing/ASCII escapes,
  optional Node, and Web readiness requirements.
- Python's version remains a diagnostic. Rust probes `python3 --version`;
  absent/broken Python produces an error object in `python` instead of preventing
  the command from starting. Neither build nor doctor requires an interpreter.
- Intentional fix beyond Python parity: exact manifest selection canonicalizes
  both the requested path and Cargo's reported manifest path, falling back to each
  raw path if canonicalization fails. Symlink spellings such as macOS /var or
  /tmp now select the intended package in multi-member workspaces.
- Deliberate CLI naming change: help and text argument errors say `usage: rake`
  and `rake: error:` instead of `dev.py`. The remaining help/error text is unchanged.
- OS errors and malformed JSON/TOML use Rust library diagnostics rather than
  emulating Python exception text. Unusual integer syntax/values outside i64
  use Rust argument parsing. Ordinary validated flags retain the old messages.
- The Unix gate remains in this phase because process ownership uses Unix
  groups. Existing portable native distribution commands are unchanged.
  Windows/process portability and script removal remain later-phase work.

## Test mapping

| Existing script coverage | Rust coverage |
| --- | --- |
| test-supervisor: doctor required targets/optional Node | doctor_matches_required_targets_and_optional_tools_without_python |
| test-supervisor: build features | web_build_selects_artifact_copies_assets_and_preserves_explicit_features |
| test-supervisor: first lockfile | first_lock_resolution_and_subsequent_metadata_stay_locked |
| test-supervisor: output replacement/failure | web_build_keeps_authored_index_and_rolls_back_failed_replacements |
| test-supervisor: owned process cleanup | cancellation_returns_zero_cleans_descendants_and_keeps_published_output |
| test-bundle: native build layout | real_desktop_build_does_not_execute_python |
| test-bundle: input validation | build_rejects_unowned_output_overlap_and_symlink_markers; web_build_rejects_links_collisions_bad_metadata_and_loader_mismatches |
| test-bundle: invalid target options | parser_preserves_errors_flags_and_doctor_validation_order |
| Symlinked workspace manifest selection (intentional fix) | symlinked_manifest_selects_exact_workspace_package |
| CLI program name (deliberate naming change) | help_and_argument_errors_use_rake_program_name |
| Artifact discovery/error paths | web_build_reports_missing_artifacts_and_ambiguous_packages |
| Metadata bounds/process errors | project_build::processes::tests (3 tests) |

Existing scripts/tests are retained. Rust CLI tests use shell tool doubles for Web
and a real Cargo console application for desktop; they require no Python, Node,
GPU or wasm-bindgen installation.

## Initial verification at 9940552 (Linux devbox, Rust 1.98.0)

All Cargo invocations used `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`.

- `cargo test -p rake-app-cli --test project_build --test distribution_commands --test distribution_bundle --test distribution_plan`:
  25 passed.
- `cargo test -p rake-app-cli --bin rake project_build`: 3 passed.
- `cargo clippy -p rake-app-cli --bin rake --test project_build -- -D warnings`: passed.
- `rake format` on changed Rust files, `cargo fmt --all -- --check`, then
  `rake lint`: passed;
  repository lint reports 14 existing non-denied diagnostics.
- Direct differential validation against unchanged dev.py: 23 byte-identical
  doctor/help/argument cases; 9 Web cases with matching status, stdout/stderr,
  generated bytes and file modes.
- `python3 scripts/test-supervisor.py`: 12 passed, 1 skipped (Node unavailable).
- `python3 scripts/test-web-deployment.py`: 5 passed.
- `python3 scripts/test-preview.py`: 18 passed.
- `python3 scripts/test-dev-bridge.py`: 4 passed.
- `python3 scripts/test-bundle.py`: 5 passed, 1 skipped (macOS dylib test);
  the real Web bundle test fails because matching wasm-bindgen 0.2.129 is absent.
- Full `cargo test -p rake-app-cli --bin rake`: 189 passed, 2 ignored, and
  the same three CDP failures
  present before this change: console tests
  `marked_mirrors_are_hidden_live_but_retained_after_runtime_failure`,
  `process_output_has_its_own_clock_cursor_and_loss`, and session test
  `evaluation_errors_are_visible_exceptions_and_crash_output_survives_dead_ipc`.
  Their process-output fixture is rejected as not a private owned regular file.
  A private-umask rerun has the same failures. No existing tests were changed.

No macOS GUI/GPUI, native-preview, real wasm-bindgen/browser, or Windows run.
No packages/toolchains installed. Devbox free disk remained above 36 GB.
