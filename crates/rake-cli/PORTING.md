# Project toolchain port: phase 1

`doctor` and the legacy one-shot `build` adapter run in Rust. Existing native
distribution modes keep their dispatch. The embedded scripts continue to support `dev`, Web `bundle`, preview and
serving. The native-review landing routes `inspect` through the existing Rust
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
