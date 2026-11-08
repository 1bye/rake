# Project toolchain port

## Phase 6: embedded Python removed

Closes #42. Base: `codex/port-phase5-preview` at
`0b11369e997a4f346d56c045382f63568e238128` ([phase 5](https://github.com/1bye/rake/pull/54)).
Phase 4 (`49633f29`) and phase 5 passed gate, portable, Windows desktop, Ubuntu
desktop and macOS native; extended was skipped on normal PR events.

### Current implementation and differences

No CLI runtime path needs Python. Delete all eight embedded .py helpers and
project.rs, including interpreter launch/extraction and its environment hook.
Materialize only preview.swift and the embedded DevTools JS/HTML for their owned
services; serve preview assets/bootstrap directly from embedded bytes. **Swift
capture, preview HTML/JS/CSS and other non-Python resources remain embedded.**
Remove the unused Python-server supervisor field. Browser candidates honor quoted
BROWSER commands and %s, then use open, xdg-open or cmd /c start. No shell is used
for BROWSER commands unless explicitly requested by the command itself.

Drop doctor’s `python` field: no built-in runtime still needs it. Preserve the
other diagnostic fields and their order, optional Node, Web readiness and exit
codes. Remove the interpreter prerequisite from CLI help, docs and generated
README. Repository-only verification scripts still use Python. build-web.py now
uses the Rust build command, including its output ownership/rollback checks;
move an old unowned dist directory before first using this wrapper. Web platform
fixtures use real Rust bundling, and the routing GUI suite uses the Rust broker.

Unix process groups and merged output retain their contracts. Windows tools use
anonymous pipes and kill-on-close Job Objects. Assign a suspended child before
resuming it, so descendants cannot escape initial enrollment. Preserve bounded
metadata/output and real missing-tool versus permission diagnostics. Web URL,
identity and ZIP paths use slash separators; Windows ZIP timestamps use the CRT
and regular files use mode 0644. The Windows Node agent uses a temporary module
rather than putting its embedded source on the bounded Windows command line;
it inherits console streams, returns the child exit code, owns its Job through
Ctrl-C and removes the module before returning. Unix retains exec semantics.
No dependency/toolchain versions changed: shlex 2 and windows-sys 0.61 were
already locked and are now direct dependencies where needed.

### Test conversion and coverage

Before deleting Python, compare all 30 preview results and exact request
envelopes, then capture the reference output in
[preview-python-reference.json](tests/fixtures/preview-python-reference.json).
The same corpus now runs by default as Rust coverage, with no interpreter or
new ignore. The two remaining default ignores predate this port.

Keep original Python suite test names and real HTTP, native, Web and CLI
assertions. Replace deleted-helper internal assertions with the named Rust
contracts in [ported-contract-tests.json](../../scripts/ported-contract-tests.json).
The runner requires actual named test execution and rejects zero-test/ignored
results. Add direct favicon/idempotency, input/base-path/metadata, private UTF-8
checkpoint and relative-program permission-error cases so coverage is retained.
Native GUI preview uses a small bounded test-only Unix JSON client, without
retaining the production Python broker. Move deployment-internal gate coverage
to the existing Rust workspace suites on Linux, Windows and macOS; no contract
or assertion is removed. The raw oversize-upload probe now reads a bounded
complete HTTP status line before asserting 413, after Linux delivered a partial
first recv. No timing bound or expected status was relaxed.

### Verification

Mac: Rust 1.98.0 / Python 3.14.4 first on PATH / Node 26.4.0 / Chrome 156,
shared /tmp/rake-port-target, dev/test debug info disabled, incremental off:

- Full CLI: **354 passed, 0 failed, 2 existing ignored** (256 unit + 98 integration).
- Clippy all-targets -D warnings, Cargo fmt, changed-file rake format and diff
  checks passed. Lint: **725 Rust files, 14 existing diagnostics, 0 denied**.
- Supervisor **14**, bridge **5**, runtime broker **9**, preview **18**,
  deployment **10**, embedded DevTools **1**: all passed. Converted internal
  cases actually execute their mapped Rust tests; HTTP cases use the real CLI.
- Matching temporary wasm-bindgen 0.2.129: bundle **7**, loading **1**,
  real Chrome crash **17**, real browser platform **9**: all passed.
- Native preview passed repeated clicks, fractional scrolling, source selection,
  blur/opaque toggles and stale rejection. No StaleGeneration on this run.
- Updated routing GUI test: **30 boundaries**, real layout/styles/source,
  replacement/stale IDs and Rust HTTP parity passed. build-web.py built real Notes.
- Live Web dev: built/published/fetched page/bootstrap and native development
  state, no Python child, SIGTERM exit 0 and **zero owned descendants** afterward.
- Live desktop --preview --cdp: built/published/fetched page, actual JPEG and
  WebSocket state, real source selection and pointer move over Rust HTTP, CDP
  discovery and agent snapshot; notes-view live/saved each **100 nodes**.
  Use isolated Notes data and reveal its own notebook window before capture.
  SIGTERM exit 0, **zero owned descendants** and removed owned session.

Additional capture repeats against an occluded, not-yet-drawn Notes window got
only keepalives; revealing that same owned application through its runtime API
made it visible/drawn and the complete capture/input check passed. No capture
assertion or production behavior changed. An initial full-run formatter check
read the unstaged deleted project.rs; stage the intended file set before its
successful repeat. Temporary harness import/ZIP API fixes preserve assertions.

Devbox: Rust 1.98.0 / Python 3.12.3, umask 022, debug info and incremental off:
**350 passed, 0 failed, 2 existing ignored**; Clippy, Cargo fmt and lint passed
(725 files, 14 existing diagnostics, 0 denied). Bridge **5**, deployment **10**
passed. Supervisor **13 passed / 1 skipped**: only Node syntax checking skips
because Node is not installed. No Node/GPUI dependencies or other tools installed.

Initial phase-6 CI at a562114: gate, portable, Linux desktop and macOS native
passed; Windows desktop failed the fresh console-build regression. Its old
Windows-only dispatch forced the raw --locked adapter, bypassing first-lockfile
creation. Remove that shortcut and extend the existing installed-layout adapter
to Windows ZIP: preserve the executable, assets and installation descriptor in
application/, report the .exe and Windows manifest identity, and reuse native
resource/manifest staging without downloading an installer. Keep explicit native
distribution builds on their existing adapter. Strengthen the real portable build
test with assets, owned rebuild and unowned-output refusal; add an all-host Windows
layout/manifest regression. Replacement Windows CI is pending.

At 67049f9 the Windows test reached native staging and correctly rejected the
fixture's missing rake-build integration. Keep that existing production guard.
Use real rake-build/build.rs and rake-resources dependencies in the fixture on
every host, and make its installed executable read the packaged asset from an
unrelated working directory. Preserve the fresh-lockfile, output ownership,
rebuild, archive/manifest and asset assertions. Windows installed builds retain
the native packager's existing rake-build requirement; explicit build --native
remains the raw Cargo executable adapter.

Fixture-correction repeat: Mac and Linux each passed all **3** portable CLI tests,
all-target Clippy and Cargo fmt; Mac rake format passed. At 67049f9, gate,
portable, Ubuntu desktop and macOS native passed again. Windows CI must still
confirm the corrected fixture with real build-script resource integration.

Follow-up validation: Mac full CLI repeat **354 passed, 0 failed, 2 existing
ignored**, portable build tests **3**, compatibility tests **2**; Linux portable
tests **3** and compatibility tests **2**. Both machines passed all-target CLI
and distribution Clippy. Mac Cargo/rake format and lint passed (725 files,
14 existing diagnostics, 0 denied).

A Windows-only
Job test exercises real descendants and metadata bounds; portable CLI tests
exercise doctor, a real console build with spaces, Web bundling with compiled
Rust tool doubles, portable URLs/ZIP names and Windows agent module cleanup.
A separate phase-5 extended dispatch reached Dylint and reported 863 earlier-code
spacing locations, zero from phases 4/5. This differs from #51’s tool-install
failure and remains reported rather than suppressed. Extended does not run on
normal PR events; do not claim a phase-6 extended pass.

### Remaining Windows work — explicit limited gate

Doctor, build, native/Web bundle and the TCP/Node agent are portable. Keep dev,
inspect/ctl, CDP and native source access gated where they use Unix facilities.
Full support still needs:

- A Windows native IPC endpoint/client/server (named pipes or equivalent),
  cancellable bounded exchanges, and the matching advertisement/session layout.
- Owned/private discovery and source/output validation using Windows ACLs,
  identities and reparse-point checks; do not weaken Unix UID/mode confinement.
- Supervisor app/CDP/companion lifecycle, readiness and output readers on Windows,
  reload/checkpoint cancellation and shutdown/descendant tests using Job ownership.
- Windows CDP/native inspection/source adapters and platform GUI validation.
  Preview capture remains macOS-specific Swift; a Windows capture adapter would
  need the same frame/input/generation bounds and lifetime rules.

This is the narrower gate explicitly permitted by the task, not a claim of
Windows dev/inspection support. No follow-up issues were opened. Keep all PRs
as drafts; retain worktrees for review fixes. Final task cleanup removes shared
build outputs, temporary wasm-bindgen and owned sessions/processes after CI.


## Phase 5: native local preview

Refs #42. Base: `codex/port-phase4-broker` at
`e0ea92fe086987b9b5efc32440238f0cbbcaf8a0` ([phase 4](https://github.com/1bye/rake/pull/52)).
Phase 4 and its accepted-socket follow-up passed gate, portable, Windows desktop,
Ubuntu desktop and macOS native; extended was skipped.

### Implementation and protocol

`project_build/preview/` owns expiring capture references, exact-generation
selection/hover/grab, bounded annotation paging, native video fanout, capture
process lifetime and authenticated RFC 6455 controls. `serve.rs` handles preview
POSTs, assets and WebSockets natively; `dev/services.rs` compiles the unchanged
Swift helper and constructs preview. No Python serving backend or proxy starts.
Remove the intermediate phase-4 NativeBroker callback; standalone retained Python
helpers remain available for reference comparisons until phase 6.

Preserve both codecs, epoch/window identity, 64 references with five-second expiry,
finite content coordinates, ties-to-even viewport geometry, exact generations,
non-retrying inputs with null keys, 33 annotation targets, 32 pages/8192 offset
and 1.5-second lookup budget. Preserve masked, unfragmented text controls,
origin/subprotocol token authentication, 16 KiB input, 256 KiB output and 250 ms
metadata updates. Video retains 120 packets / 8 MiB, 2 MiB payload / 4096-byte
metadata bounds, configuration replay and keyframe recovery after gaps or age
above 150 ms. Reserve viewers before headers and release encoders after the last
viewer, including header disconnects. Capture close retains the two-second grace
before terminating its owned process group.

**Swift capture and preview JS/CSS/HTML remain embedded non-Python resources.**
Python extraction and browser selection remain for phase 6; the Unix gate is
unchanged. No preview feature is dropped. Preserve the existing HTTP/1.0 101
WebSocket response and earlier native JSON ordering/UTF-8 escaping/Server-header
differences. Drain helper stderr continuously, retaining only 4096 diagnostic
bytes, rather than risking a full pipe before reading it on exit.

### Coverage mapping

Existing Python internal tests remain present. The HTTP case in test-preview.py
now uses the real Rust CLI by default; `RAKE_TEST_PYTHON_BROKER=1` selects the
reference. Removed Python-backend readiness/fatal-exit cases become native
listener shutdown and capture-compiler failure tests. Existing CDP/Node readiness
coverage remains; no assertion is silently removed or weakened.

| Python contract | Native coverage |
| --- | --- |
| exact selection/source/window | `selection_keeps_source_and_exact_window_without_mutation` |
| text promotion, source ancestry and copy text | `grab_promotes_text_and_collects_pinned_ancestors_and_descendant_text` |
| lightweight hover / changed capture | `hover_is_lightweight_and_expired_context_never_uses_another_capture` |
| stable/exact annotation targets | `annotation_resolution_follows_stable_ids_and_keeps_exact_targets_scoped` |
| pinned pages and window generations | `annotation_resolution_pins_pages_and_rejects_other_window_generations` |
| nonadvancing pages / hidden layouts | `annotation_resolution_rejects_nonadvancing_pages_and_masks_hidden_targets` |
| invalid targets / expired references | `invalid_annotation_targets_and_expired_references_never_reach_runtime` |
| one-shot click / fractional scroll | `click_is_submitted_once_without_retry_key_and_fractional_scroll_keeps_geometry` |
| invalid positions / input bounds | `invalid_expired_or_foreign_positions_never_reach_runtime`, `scroll_is_bounded_and_unsupported_input_is_rejected` |
| helper/reference cleanup | `closing_capture_reaps_its_helper_and_releases_waiting_viewers`, `preview_state_bounds_references_and_changes_epoch_only_for_window_geometry` |
| packet bounds/truncation | `truncated_and_oversized_capture_frames_fail_explicitly`, `capture_queue_bounds_both_packet_count_and_retained_bytes` |
| joining at a keyframe / header disconnect | `video_join_starts_at_keyframe_and_releases_viewer_on_disconnect`, `disconnect_during_video_headers_releases_encoder` |
| WebSocket framing/limits | `masked_text_and_control_frames_reject_fragments_unmasked_and_oversized_input`, `outbound_frames_use_all_length_encodings_and_keep_the_response_bound` |
| HTTP auth/assets/control/shutdown | `serve::tests::native_preview_assets_authentication_control_replies_and_shutdown` |
| preview flags | existing `project_build` argument and `dev::tests` parser coverage |

### Verification

Mac: Rust 1.98.0, Python 3.14.4 first on PATH, Node 26.4.0; shared
`/tmp/rake-port-target`, dev/test debug info disabled and incremental off.

- Full `cargo test -p rake-app-cli --locked --no-fail-fast`: **340 passed,
  0 failed, 3 ignored** (245 unit and 95 integration). Two ignores predate this
  port; the new manual Python comparison separately passed all **30 exact
  result/request-envelope cases** with Python 3.14.
- All-target Clippy `--locked -- -D warnings`, Cargo fmt and changed-file
  rake format checks passed. `rake lint`: **721 files, 14 existing diagnostics,
  0 denied**. `git diff --check` passed.
- Supervisor **14**, dev bridge **5**, runtime broker **9**, preview **18**,
  Web deployment **10**, embedded DevTools **1**: all passed.
- Python-reference preview: **18 passed**. The original Python internal assertions
  also remain in the default script suites; earlier phase-4 broker comparisons
  retain their recorded results below.
- With authorized temporary wasm-bindgen 0.2.129: bundle **7 passed**, real
  Fieldnotes loading passed, Chrome 156 crash suite passed **17 scenarios**.
- Native preview passed repeated clicks, bidirectional fractional scrolling,
  Rust-source selection, blur/opaque toggling and stale-reference rejection.
  No StaleGeneration occurred on this run; the previously known intermittent
  race remains recorded, without evidence that this port causes it.
- Live Web `rake dev -p rake-notes --target web --no-open --json`: built,
  published, fetched immutable/bootstrap-injected page and native development
  state; SIGTERM exit 0, stopped record and owned session cleanup.
- Live desktop `--preview --cdp --no-open --json`: built/published, fetched page,
  native preview state, actual JPEG frame, authenticated WebSocket state, CDP
  discovery and agent snapshot; notes-view live/saved each **100 nodes**.
  SIGTERM exit 0 and stopped record; use an isolated Fieldnotes data directory.

Earlier Mac runs exposed inherited O_NONBLOCK (fixed and regression-tested in
phase 4) and a new fixture setting a timeout after RST (move timeout setup before
RST, without weakening cleanup assertions). The first final supervisor attempt
timed out while concurrent checks held the shared Cargo target; its serial repeat
passed all 14 unchanged cases. The initial live-video probe rejected a valid
keepalive before the image; the corrected probe waited for JPEG and passed.
Temporary installation build outputs were removed immediately after installation.
The user subsequently removed the 40 GB disk floor; no unrelated files/processes
were cleaned or stopped.

Devbox, Rust 1.98.0 / Python 3.12.3, umask 022, debug info and incremental off:
full CLI **336 passed, 0 failed, 3 ignored**; focused preview **18 passed** and
manual Python differential **30 passed**. Clippy, Cargo fmt, rake format and lint
passed (721 files, 14 existing diagnostics, 0 denied). Bridge **5**, reference
preview/broker **18 / 9**, deployment **10** passed. Supervisor: **12 passed,
1 skipped, 1 error** because Node is absent; its real CLI lifecycle case passed.
No Node or GPUI system dependencies were installed on devbox. Its default umask
002 exposes an unchanged archive mode assertion; the full repeat passed with 022.
Interpret new helper doubles with the installed shell to avoid Linux ETXTBSY
from concurrent forks retaining writer descriptors; real Swift launch is direct.

Phase-5 dispatched CI at `59f5dec` passed gate, portable, Windows desktop,
Ubuntu desktop and macOS native. Extended installed spacing tools successfully,
then failed `scripts/check-spacing.py`; this is distinct from #51's install error.
The captured diagnostics identify 283 phase-4-owned and 420 phase-5-owned
blank-line locations plus 863 locations attributed to pre-phase-4 code. Fix the
port-owned locations on their phase branches, carry the lower fix forward by
cherry-pick, and keep the earlier baseline warnings visible. These Rust changes
insert blank lines only (`git diff --ignore-blank-lines` is empty). Mac preview
unit tests, Cargo fmt, all-target Clippy and diff checks pass; Dylint itself is
verified in dispatched CI because additional local installs are not authorized.
Fresh CI is pending on the follow-up heads. Phase 6 removes retained Python/extraction,
updates prerequisite docs and browser opening, and resolves Windows boundaries.

## Phase 4: native runtime broker and development bridge

Refs #42. Base: main at `85f13c8`. `project_build/broker/` owns browser leases,
private discovery, runtime forwarding, source-opening invocation, CDP discovery
and output reads. `serve.rs` handles authenticated bridge/broker POSTs directly.
Web sessions no longer launch `serve-web.py`. Keep the Python sources available
for reference comparisons and phase-5 preview; extraction/removal belongs to phase 6.

### Protocol and ownership

- Retain exact runtime envelopes, request/retry keys, JSON result envelopes,
  action names, status codes, 1 MiB response frames and the 1 MiB minus 1024-byte
  HTTP input limit. The bootstrap bytes, discovery/session fields and public
  origin advertisements stay compatible with `rake cdp`, `rake agent`, browser
  runtimes and embedded DevTools.
- Retain constant-time token authentication, numeric loopback binding, Host and
  Origin checks, canonical runtime IDs and validated loopback HTTP forwarding
  with redirects forbidden. HTTP fields decode as ISO-8859-1, matching Python,
  so non-ASCII tokens receive 403 rather than a reset.
- Discovery/output reads use no-follow, nonblocking opens, descriptor metadata,
  current effective UID, private regular-file checks and bounded reads. Native
  access requires the control-only origin and an exact private control-directory
  socket with owned/private socket metadata. List at most 128 sorted discovery
  candidates; never expose endpoints/tokens to the page.
- Browser state owns at most 32 leases and 128 tickets (32 per runtime), preserves
  insertion-ordered ticket delivery/redelivery, 30-second leases, 60-second ticket
  expiry, 2-second polls, 10-second client timeouts and busy-reload retry delays.
  A client timeout retains the ticket and is not cancellation or approval.
- Captured source targets accept only bounded exact element IDs and boundary,
  style or handler indices. Invoke the existing `ctl inspect source` command with
  the session control directory/source root; that command keeps Rust-file and
  workspace confinement. Ignore supplied file paths. Own the opener process group,
  bound both pipes and cancel/reap it on shutdown.
- Server shutdown cancels HTTP forwarding, wakes polls/clients, removes owned
  advertisements, closes connections and joins workers. Native connect/write/read
  steps use cancellable five-second deadlines, including full socket backlogs.
  Check stopping under the
  connection-registration lock to avoid accepting an untracked late reader.

### Complete intermediate preview boundary

Only preview sessions start the retained Python server. A small authenticated
`NativeBroker` callback supplies its `Preview` with native development state,
instance discovery and runtime requests. The Rust origin forwards only preview
routes (including the existing binary/video and WebSocket transport); no Python
bridge lease/ticket state is instantiated in that mode. Its private backend and
shared token belong to the session. Standalone Python serving remains unchanged
unless the internal `RAKE_NATIVE_BROKER_URL`/`RAKE_NATIVE_BROKER_TOKEN` are set.
Swift capture and the preview JS/CSS/HTML assets remain embedded resources.

### Tests and Python mapping

The HTTP suites now start real Rust CLI dev origins by default using
`scripts/broker_test_server.py`: Cargo/wasm-bindgen/Swift are tool doubles, while
HTTP, authentication, broker, terminal clients and Unix peers are real. Set
`RAKE_TEST_PYTHON_BROKER=1` to run the same assertions against the reference server.
Original Python internal tests stay present; none are removed or weakened.

| Original coverage / contract | Native coverage |
| --- | --- |
| bridge owned roundtrip, redelivery, CLI and private discovery | `test-dev-bridge.py` HTTP tests run against Rust; `owned_protocol_roundtrip_redelivery_private_discovery_and_cleanup` |
| capacity and expired browser discovery | `capacity_expired_leases_and_tickets_keep_timeout_delivery_semantics` |
| native forwarding and credential hiding | `test-runtime-broker.py` HTTP tests run against Rust; `native_forwarding_keeps_the_exact_envelope_and_rejects_foreign_or_public_sockets` |
| exact source invocation and captured target validation | `source_targets_accept_only_exact_element_ids_and_captured_indices`, `source_opener_receives_exact_arguments_control_directory_and_source_root` |
| CDP disable/default, URL constraints, redirect rejection, response bounds | `loopback_endpoints_reject_credentials_paths_aliases_and_redirect_targets`, `external_web_and_cdp_forwarding_reject_redirects_and_bound_response_bodies` |
| external Web forwarding | `external_web_forwarding_preserves_credentials_and_envelope_only_at_validated_origin` |
| successor metadata isolation | `native_successor_metadata_belongs_only_to_the_supervisor_runtime` |
| Web origins cannot list/access native IPC | native forwarding test plus retained Python internal assertion |
| process output bounds and cursor/privacy | existing `dev::tests::process_output_matches_the_private_ring_and_utf8_byte_bounds`; `output_is_private_bounded_and_cursor_filtered_without_inventing_causal_ownership` |
| secure bounded file reads | `private_reads_reject_links_public_files_fifos_and_oversized_frames` |
| native reply bounds and socket cancellation | `native_reply_bounds_and_shutdown_cancel_pending_socket_work` |
| reload busy retry, cancellation and opener output bounds | `reload_busy_retries_and_shutdown_wakes_pending_clients`, `source_output_overflow_is_bounded_and_stopped_brokers_do_not_launch_openers` |
| no Python Web server, bind errors and helper lifecycle | new `project_dev::web_serves_without_python_or_node_and_reports_occupied_listener`; existing readiness/fatal-exit tests now exercise the retained preview backend |
| HTTP JSON size/type/shape and build-ID validation | additional `test-dev-bridge.py::test_json_frame_bounds_shape_and_action_validation`, run on both implementations |

A new oversized-upload reference check reproduced Python's early-response reset;
compare declared oversize lengths without uploading the body, asserting the same
413 deterministically. Rust also passed the actual oversized-body check. The
existing early-error drain regression remains. The initial native token check
caught UTF-8-only header decoding; fix it with HTTP's Latin-1 decoding. A new CDP
test caught optional-discovery fallback hiding redirect rejection; preserve the
explicit error separately from unavailable discovery. The first full run caught
an incorrect Content-Length in the new fake Web peer; correct its wire length.
Three supervisor fixtures timed out under concurrent validation on that run;
the complete repeat passed all 15 without assertion or timeout weakening.

### Differences and retained work

- Preserve prior native JSON key ordering/OS-library diagnostics and static Server
  header differences. Tokens, PIDs, ephemeral ports and timing naturally differ.
  JSON uses UTF-8 rather than Python's ASCII escaping; wire frames stay bounded.
- Source-opener output was previously captured without a bound before truncation.
  Both native pipes now stop at 1 MiB; overflow reports the existing unavailable
  opener error and reaps the process group. Source errors still expose at most
  2048 characters. Shutdown explicitly cancels outstanding broker work.
- Reuse workspace `rake-http` for deadline/cancellation-aware HTTP with redirects
  disabled, `async-io` for the existing client's native socket adapter, and already
  locked `subtle` for constant-time token comparison.
  No new dependency versions or toolchain installs. Temporary wasm-bindgen
  0.2.129 is the sole testing-tool installation and is removed at handoff.
- The Unix gate remains necessary for the existing supervisor/process and native
  socket implementation. Windows scope, native browser selection and prerequisite
  documentation remain phase 6. No feature is dropped and no Windows claim is made.

### Verification: macOS, Rust 1.98.0 / Python 3.14.4 / Node 26.4.0

Use `/tmp/rake-port-target`, dev/test debug information disabled, incremental off,
Python 3.14 and temporary wasm-bindgen first on PATH. Linux/devbox not used.

| Check | Result |
| --- | --- |
| `cargo test -p rake-app-cli --locked --no-fail-fast` | **320 passed, 2 existing ignored, 0 failed**: 225 unit + 95 integration |
| CLI all-target Clippy `--locked -- -D warnings` | Passed |
| `cargo fmt --all --check`, changed-file `rake format --check`, diff whitespace | Passed |
| `rake lint` | Passed: 714 Rust files, 14 existing diagnostics, 0 denied |
| Supervisor / bridge / broker / preview / Web deployment / embedded DevTools suites | **14 / 5 / 9 / 18 / 10 / 1 passed** |
| Reference bridge / broker suites | **5 / 9 passed** with `RAKE_TEST_PYTHON_BROKER=1` |
| Direct Python/Rust differential | **36 normalized JSON/status cases passed**; both advertisements/sessions cleaned |
| Bundle suite with matching wasm-bindgen 0.2.129 | **7 passed, 0 failed** |
| Web loading | Passed real Fieldnotes module-graph resolution and startup error display |
| Web crash | Passed **17 real Chrome 156 scenarios**, including Rust panic/sink, privacy, cross-tab delivery, UTF-8 limits and recovery |
| Native preview | Passed repeated click, fractional scroll, Rust-source selection, blur/opaque toggle and stale-reference checks; no StaleGeneration on this run |
| Live Web dev | Built/published Fieldnotes, fetched injected immutable page/bootstrap and native development state; no Python server; SIGTERM exit 0 with clean stopped records/advertisements/owned descendants |

Desktop `rake dev -p rake-notes --preview --cdp --no-open --json` also passed:
Swift compilation, real native publication, authenticated preview state, RFC6455
upgrade/state delivery through the native router and a `rake agent` snapshot.
Live and saved notes-view captures each passed with **100 nodes**. SIGTERM exited
0; stopped records, advertisements and all observed owned descendants were clean.
CI results belong in the PR body. After the final full run, the fixture cleanup
check repeated all **15 project_dev tests**, passing with early-startup sessions
removed by their own PID and creation time. The first desktop probe reached publication before runtime discovery;
wait explicitly for its advertisement before querying. An initial WebSocket probe
expected HTTP/1.1, while the retained Python transport correctly emits its existing
HTTP/1.0 101 response; the probe now checks that unchanged response.

Keep `/tmp/rake-port-p4` for review fixes. Reclaim completed Wasm artifacts after
those suites to preserve the requested 40 GB free-space floor. The live checkout
is untouched apart from fetch/worktree registration. No merge, main push,
force-push, rebase, amend, issue comment or label.

### Follow-up: CI for stacked base branches

Allow existing pull-request CI on the phase-4 and phase-5 base branch names as
well as main. Keep jobs, assertions, permissions and extended's existing event
condition unchanged. The manual phase-5 run at `1a9fb99` verified that all 703
port-owned spacing locations are fixed; extended retains 863 earlier-code
locations and zero attributed to this port. Its branch-dispatched status was not
reported on the stacked PR dashboard, so use normal PR events for subsequent
stack checks rather than repeated manual dispatches.

### Follow-up: spacing discovered by dispatched CI

Phase-5 manual CI passed gate, portable, Windows desktop, Ubuntu desktop and
macOS native, but extended reached the spacing checker and failed. Tool
installation succeeded; this is distinct from the installation problem in #51.
Apply the 283 reported blank-line insertions owned by phase 4 on its own branch;
no non-whitespace Rust change. Mac: broker tests **14 passed**, Cargo fmt and
whitespace/diff checks passed. Carry this commit forward without rewriting any
pushed history. Older spacing warnings outside phases 4/5 remain a reported
baseline failure, with phase-5 findings recorded in its PR.

### Follow-up: accepted connection mode

The phase-5 native WebSocket regression exposed macOS inheriting O_NONBLOCK from
our nonblocking listener. Explicitly restore blocking mode on accepted worker
sockets before timed HTTP reads; preview proxying still selects its existing
nonblocking forwarding mode. The new regression splits both headers and JSON
across a bounded read timeout and requires successful authenticated completion.
The phase-5 WebSocket case failed on the Mac before this change and passed after
it. A real Mac CLI check splitting authenticated headers and JSON also passed.
Devbox, Rust 1.98.0 with umask 022: full CLI **317 passed, 2 ignored**,
all-target Clippy `-D warnings`, Cargo fmt, rake format and rake lint passed.
The added Rust segmented-request regression is included in that full run.
Latest CI is recorded in the PR body. This fix is a
new commit on the phase-4 branch; pushed history is preserved.

### Remaining phases

- Phase 5: port `preview.py`/`preview_socket.py` and retire the compatibility server;
  retain Swift capture and browser resources.
- Phase 6: delete embedded Python/extraction/launch paths, replace Python browser
  opening, update prerequisites/doctor and resolve platform gates/Windows scope.

## Phase 3: Web serving, bundles and offline payloads

Refs #42. Based on `codex/port-phase2-web` at
`7e333854742b09aa25e80d3de5219b67de715cf2`. The Web bundle adapter now runs in Rust,
including release/debug selection, explicit deployment overrides, staging/rollback,
ZIP packaging, deployment/version metadata, immutable offline payloads, checksums
and nginx guidance. It reuses phase 1's builder, input validation and favicon logic.
A fixture with only Cargo/wasm-bindgen tool doubles on PATH verifies Web bundles
without Python. Native distribution dispatch remains with the existing coordinator.
Attached/abbreviated target options and the final target selection reach the Web
adapter through the shared option inventory; standalone Web configs retain their
explicit unsupported-operation diagnostic.

### Serving and staged control boundary

`dev/services.rs` binds the native loopback listener on the requested port. Rust
owns static assets, MIME types, HEAD/cache checks, directory redirects/listings,
HTML app-shell fallback, bootstrap injection, immutable numbered build paths,
Referer-based absolute-import selection and traversal confinement. The bootstrap
is retained byte-for-byte as `scripts/web-bootstrap.js`, embedded as JavaScript.

The retained Python helper binds a separate ephemeral loopback port and handles
only requests forwarded for the phase-4 broker/bridge and phase-5 preview. The
native router checks the public Host/Origin before forwarding; it rewrites them
to the private backend authority and forwards bounded JSON bodies and the full
HTTP/WebSocket response stream. An internal `RAKE_PUBLIC_BRIDGE_URL` override
keeps browser advertisements on the public native origin. Private tokens,
endpoints, session fields and browser protocols are unchanged. Every request,
listener, connection and helper remains owned by the existing supervisor. Both
server layers bound concurrency to 64; shutdown closes retained sockets and joins
owned workers. The original helper stays embedded until phases 4–6 retire its
remaining responsibilities; this phase does not claim to remove Python serving
of preview resources or broker responses.

### Implementation and coverage mapping

| Original | Rust owner / new coverage |
| --- | --- |
| `bundle.web_deployment`, `distribution`, `sha256` | `bundle.rs`; CLI `web_bundle_preserves_manifest_archive_offline_payload_and_rollback` checks defaults/overrides, release/debug, ZIP member bytes/modes/timestamps, manifests and rollback |
| `web_offline.prepare`, service-worker template | `offline.rs`, unchanged embedded `web-service-worker.js`; `offline_builds_pin_assets_and_keep_router_root_separate`, `online_bundles_have_version_checks_without_changing_asset_layout` |
| Production reserved names and managed namespace rewriting | `generated_names_and_development_metadata_cannot_collide_with_inputs`, `deployment_overrides_rewrite_only_managed_namespaces_and_skip_bindings` |
| `AppHandler.translate_path` | `serve.rs`; `immutable_builds_survive_publication_and_referer_pins_absolute_imports`, including explicit default ports, foreign Referers, traversal and escaping symlinks |
| `send_head`, GET/HEAD, app-shell fallback, cache/redirect/listing | `static_serving_keeps_mime_head_cache_redirects_and_html_fallback` |
| Bounded listener and shutdown | `shutdown_releases_partial_request_readers_and_checks_control_host` |
| Early HTTP authentication errors with request bodies | `rejected_post_bodies_preserve_the_http_error_instead_of_resetting_the_connection`; differential foreign-Origin rejection |

All original scripts remain. No internal Python coverage was silently removed or
weakened. Rust deployment tests additionally map the original deployment suite's
offline identity/layout, online version checks, generated-name collisions and
namespace behavior; phase-1 config validation tests remain in place.

### Differences and dependencies

- Build IDs remain deterministic 64-character SHA-256 identities. The implementation
  salt is now `offline.rs` instead of `web_offline.py`; IDs and the corresponding
  immutable paths change across this implementation version. Differential checks
  normalize only those IDs and JSON object-key order. Service-worker semantics,
  cache prefix, asset quoting, version fields and registration text are preserved.
- ZIP compression is performed by the locked Rust ZIP implementation rather than
  Python/zlib. Archive bytes/checksums can differ; member contents, relative names,
  local file timestamps, Unix modes, manifest sizes and SHA-256 checksums are verified.
- The native static HTTP response identifies its server as `rake`; generated JSON
  object ordering and OS/library error wording follow the existing Rust differences.
  Request headers are bounded to 64 KiB. The bridge/preview HTTP/WebSocket protocols
  stay with their existing helpers until their native port phases.
- Add direct dependencies on already locked `zip` (same deflate-only feature choice
  as rake-dist), `percent-encoding` and `mime_guess` for their existing archive/URL/MIME
  implementations. Add the small `httpdate` crate for HTTP date parsing/formatting
  and conditional caching, including legacy HTTP date syntax. No package/toolchain
  installs; Cargo dependency resolution is recorded in Cargo.lock.
- Unix gate and Python prerequisite docs remain for phase 6. Swift and browser assets
  remain resources; no preview capture implementation is removed.

### Verification: macOS, Rust 1.98.0 / Python 3.14.4 / Node 26.10.0

Shared `/tmp/rake-port-target`, dev/test debug information disabled, Python 3.14
first on PATH. Linux/devbox not used.

| Check | Result |
| --- | --- |
| Full `cargo test -p rake-app-cli --no-fail-fast` | **305 passed, 2 existing ignored, 0 failed**: 211 unit + 94 integration |
| CLI all-target Clippy, locked, `-D warnings` | Passed |
| `cargo fmt --all --check`, changed-file `rake format --check` | Passed |
| `rake lint` | Passed: 710 Rust files, 14 existing diagnostics, 0 denied |
| Supervisor / bridge / broker / preview / Web deployment / embedded DevTools suites | **14 / 4 / 9 / 18 / 10 / 1 passed** |
| Bundle suite | **6 passed, 1 failed**: missing matching wasm-bindgen 0.2.129 |
| Web loading / Web crash suites | Built their real Wasm fixtures, then both failed the matching-loader prerequisite; no module/browser execution claim |
| Native preview suite | Passed click/scroll/source/blur/stale-reference assertions on the unchanged real GPUI fixture |
| Real Rust-supervised notes preview/CDP session | Authenticated preview-state and RFC6455 upgrade/state frame through the native router, live and saved notes hierarchy (**100 nodes**), SIGTERM exit 0 and clean stopped discovery records passed |
| Bundle differential against Python | **4 cases passed**: online/offline × path/hash routing. All normalized site file contents, nginx bytes and manifest contract fields matched; ZIP member contents, modes, timestamps, file sizes and all recorded SHA-256 checksums independently verified |
| Web lifecycle differential against Python | Normalized events, status/output and all five initial build files matched, with actual HTTP immutable assets, failed build retention/recovery and SIGTERM. Browser connect advertised the public origin; authenticated poll/reload_result and foreign-Origin 403 rejection passed |

An initial post-shutdown test tried a new TCP connection to an ephemeral address;
a parallel test could reuse that address. It now verifies closure of an established
owned partial-request connection, retaining the shutdown assertion. The expanded
HTTP differential caught a real unread-POST-body reset before the 403 reached the
client; a new regression and bounded body handling fix it. An initial notes check
ran before the window drew and reported Hidden; explicitly revealing the notebook
and waiting for its hierarchy let the unchanged live/saved assertions pass.

Phase-2 CI at `7e33385`: Linux desktop and portable passed. macOS native failed in
the untouched rake-dist release test
`provider_key_changes_require_explicit_channel_reenrollment` at `release.rs:1945`;
no green full-workspace claim or baseline reproduction is made. Windows desktop
and this phase's CI status are recorded in the final PR bodies.

Free Mac disk stayed above 50 GB; completed Wasm artifacts and inactive incremental
caches were reclaimed before the floor. Keep both worktrees for review fixes and
remove the shared target/logs at handoff. The live checkout stayed read-only apart
from fetch/worktree registration.

### Remaining work

- Phase 4: replace the compatibility backend's runtime broker and dev bridge in Rust.
- Phase 5: replace preview control/capture transport orchestration; retain sensible Swift/JS/CSS/HTML assets.
- Phase 6: delete all embedded Python and extraction, remove prerequisite documentation,
  and resolve the Unix gate/Windows scope. Browser selection still uses webbrowser.

## Phase 2: Web and preview session supervision (historical)

Refs #42. All `rake dev` sessions now use the existing Rust supervisor in
`src/project_build/dev/`; there is no Web/preview supervisor dispatch to `dev.py`.
The one-shot Web bundle still uses the embedded adapter. Serving, runtime broker,
browser bridge and preview transports remain unchanged Python helpers until
phases 3–5. No Python helper or existing coverage is deleted in this phase.

### Implementation and test mapping

| Python contract | Rust owner / coverage |
| --- | --- |
| Web/preview arguments, default ports, Node 22 validation | `args.rs`; `dev_features_and_options_use_the_shared_native_supervisor`, `web_cdp_rejects_missing_old_invalid_or_failing_node_before_loading_project` |
| `Supervisor.start_server`, owned CDP, waiting document, Swift compilation, server environment/readiness | `dev/services.rs`; `web_server_readiness_is_cancellable_and_exit_is_fatal`, `web_server_invalid_readiness_fails_and_no_cdp_needs_no_node`, `preview_session_compiles_helper_before_server_and_marks_app_environment` |
| Web publication and one Node companion after the first successful build | Shared `Supervisor::publish` plus `services::start_embedded`; `web_publication_owns_server_and_single_companion_and_retains_old_build_on_failure` |
| Server/companion failure observation and process-group cleanup | Shared `Supervisor::run`/`finish`, `services::observe_services`; `web_companion_exit_is_fatal_and_invalid_readiness_blocks_publication` |
| Preview application environment and retained checkpoint/restart coordination | Shared native `launch`/`prepare`/`publish`, with `RAKE_DEV_PREVIEW=1`; existing stateful restart/Busy/restore tests retained |
| Browser selection and delayed Web/CDP opening | Owned `python3 -m webbrowser` preserving platform and `BROWSER` selection; `browser_opening_uses_webbrowser_selection_and_cdp_opens_only_the_companion` |
| CDP/Web/Node readiness framing | Shared cancellable poll reader; `readiness_accepts_unterminated_last_line_and_rejects_empty_or_oversized_frames` |
| Embedded resources | Shared `project::extract`; session-owned temporary directory removed after children stop |

The existing debounce, fingerprints, metadata refresh, build worker, supersession,
publication retry, checkpoint preparation, native replacement and output ownership
are extended rather than duplicated. Web status keeps `web_page_reload`, immutable
numbered directories, `published`, `directory`, `devtools_url` and the same events.
The bridge continues coordinating browser reloads and decisions with that private
status file. Native preview compiles the same Swift helper, starts the same
control-only origin and keeps the same HTTP/WebSocket protocols. Build diagnostics
now enter the shared output ring on every dev target, matching Python (the earlier
Rust desktop slice only captured app/reloader output).

### Differences and retained boundaries

- Preserve the prior Rust naming, canonical-manifest selection and library-error
  wording differences documented below. JSON key order, random names, PIDs,
  assigned ports and elapsed times differ naturally; field names/types do not.
- All three readiness pipes are cancellable, handle interrupted polls and accept
  a final unterminated line. Bound each readiness line to the existing 1 MiB
  control-frame limit; the old Python Web/Node reads had no size bound.
- The session now owns/removes its extracted helper directory on normal shutdown
  and errors. Retained session status/output/checkpoints/builds remain available.
- Python still runs serving/broker/bridge/preview and browser selection; only
  session supervision has moved. No new dependency or toolchain installation.
- The Unix gate remains pending phase 6. No Windows support claim in this phase.

### Verification: macOS (Rust 1.98.0, Python 3.14.4, Node 26.10.0)

Shared `CARGO_TARGET_DIR=/tmp/rake-port-target`, dev/test debug information disabled.
Python 3.14 was first on PATH for the final full run and script suites.

| Check | Result |
| --- | --- |
| `cargo test -p rake-app-cli --no-fail-fast` | **296 passed, 2 existing ignored**, no failures (203 unit + 93 integration) |
| `cargo test -p rake-app-cli --test project_dev` | **14 passed**, including seven new Web/preview/browser lifecycle cases |
| Clippy all CLI targets, locked, `-D warnings` | Passed |
| `cargo fmt --all --check`, changed-file `rake format --check` | Passed |
| `rake lint` | Passed: 707 Rust files, 14 existing diagnostics, 0 denied |
| `test-supervisor.py` | **14 passed** against the Rust CLI; original Python internal tests retained |
| `test-dev-bridge.py`, `test-runtime-broker.py`, `test-preview.py` | **4 / 9 / 18 passed** |
| `test-web-deployment.py`, Node `test-embedded-devtools.mjs` | **10 / 1 passed** |
| `test-bundle.py` | **6 passed, 1 failed**: matching wasm-bindgen 0.2.129 absent |
| `test-web-loading.mjs`, `test-web-crash.mjs` | Both built their real Wasm fixture, then failed the matching wasm-bindgen assertion; no browser/module execution claim |
| `test-preview-native.py` | Passed all assertions on the real GPUI fixture; no StaleGeneration on this run |
| Real `rake dev -p rake-notes --preview --cdp` | Swift compile, server/CDP readiness, authenticated preview-state native window, live and saved `test-notes-view.py` (**104 nodes**), SIGTERM exit 0 and stopped/clean discovery records passed |
| Python differential | **19 exact argument/status JSON and exit-code cases**; Web/CDP/real Node companion lifecycle matched normalized events, final status/output, all five generated file contents, immutable HTTP assets, failed build retention, recovery and SIGTERM |

The new Web lifecycle regression failed against the pre-port CLI before the
implementation. Initial full Cargo validation accidentally used system Python 3.9
and failed the unchanged distribution fixture's `hashlib.file_digest`; rerunning
with Python 3.14 first passed the full suite. A readiness-cancellation test fixture
initially waited for the readiness it intended to interrupt; its discovery now
also reads the double's server arguments. Assertions were not weakened.

Linux/devbox was not used. Windows/Linux desktop CI results belong in the PR body.
No packages or toolchains installed. Mac free disk remained above 50 GB (68 GiB
at the end of native verification). The live checkout was only fetched and used
to register the worktree. Keep the worktree for review fixes; remove the shared
target and verification scratch/logs at handoff.

### Remaining work

- Phase 3: Rust Web serving, deployment/bundling/offline payloads and Web bundle adapter.
- Phase 4: Rust runtime broker and development bridge.
- Phase 5: Rust preview transport and control; retain useful Swift/JS/CSS/HTML assets.
- Phase 6: remove embedded Python/extraction and prerequisite docs; resolve platform gates.
- Existing intermittent native preview StaleGeneration remains an independently
  known gap even though this native-preview run passed.

## Phase 2: native desktop supervisor slice (historical)

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
