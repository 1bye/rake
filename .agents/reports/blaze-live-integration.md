# Blaze live integration — UI timer and retained-state validation

**Input-free live polling and redraw work with the unchanged NEON engine.** Six-trial medians behind the editor are **0.830 s apply / 0.897 s redraw for `view!` text**, and **0.401 s / 0.621 s for a helper**. The uninterrupted **64-edit fill retains draft state and logical identity through every edit**, including the checkpointed restart at edit 64. Close-all/reopen and a later patch pass. The old `entry-9` caller remains unresolved; the evidence establishes a navigation operation, not an address-based identity invalidation or proven loss of the original draft.

Branch: `codex/blaze-live-rust-supervisor`, `/Users/yuriihulyk/Documents/GitHub/rake-worktrees/blaze-live`. Rebased onto fetched **`origin/main = 85f13c8feb00c5a881413a9fe85eabfe023bea7c`** before the new measurements; a later explicit fetch confirmed the same tip. Main's Rust Web/preview and serving implementation is retained. Nothing was pushed or posted upstream. The original checkout is not edited.

[Current external evidence](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/), [previous NEON evidence](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/neon/), [report before this task](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/report.before-timer.md). Raw logs, screenshots, hashes and harnesses remain outside Git.

## Implementation

- `LiveDriver` owns one GPUI `Task<()>` in the application global. A background timer wakes it about every **16 ms**, and `AsyncApp::update` polls on the UI thread. An empty poll creates no Rake outcome/event or redraw work. The task is cancelled when its owner is dropped.
- The driver has no window owner or window-close subscription. It remains alive with zero windows; repeated `install` calls reuse the existing global.
- `Applied` calls `refresh_windows()` and, on macOS, requests an occluded native frame for every open window. The existing native frame helper moved from inspection to `application.rs` and is shared by input and live updates. Layer display is deferred until after the App update to avoid re-entering GPUI while it is borrowed. No frame is forced on idle timer ticks.
- The old caller signatures were updated in routed and managed windows. Environment gates, bypass/full boundary semantics and the normal supervisor patch/restart decisions are retained.

The timer-only prototype applied patches while hidden but left the occluded rendered tree stale for more than ten seconds. That real failing check justified reusing the native frame helper; the final twelve passive trials succeed. [Timer-only failure](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/timer-only-redraw-failure.json).

The fake-source unit test drives the actual GPUI timer with **no windows or rendered frames**: three empty ticks, an `Applied` on the next tick, and no more polls after the task drops. All five live-runtime tests are exercised with `live,view-test-support`.

## Engine, toolchain and setup

The executable directory remains `/Users/yuriihulyk/Documents/GitHub/rake-worktrees/blaze-neon-tools/`. The engine source and **13 protected binary/toolchain hashes** are compared against the start of this task. No engine, toolchain, macro-server, GPUI or dependency implementation is changed. The previous [NEON lowering diff](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/neon/neon-lowering.diff) is unchanged. `pf-no-simd` stays enabled in Rake.

The normal hybrid environment remains `BLAZE_PROC_MACRO_WASM=0`, `BLAZE_PROC_MACRO_SERVER=0`, sandbox **on**, memory maximum **16G**, 64 MiB Rust stack, tier/spare off, and line-table debug information. The private tools/home/store/target from the NEON run are reused. `caffeinate -i` is used during work. No package or toolchain installation is performed.

This task does not rerun an empty-store cold gate. The previous **161.16 s** gate, zero refusals/macro failures, six-intrinsic differential probes and exact JPEG comparisons remain historical results on base `1cc5cdc`, linked in the prior report. They are not relabeled as new-base cold measurements.

## Hidden and minimized delivery, and idle cost

| Condition | Publication → apply | Evidence / limit |
|---|---:|---|
| App hidden before save, no input | **17.772 ms** | Native preview list empty; app receives `Applied` |
| Behind editor, six text edits | **25.539 ms median** | Window reported occluded before and after all trials |
| Behind editor, six helper edits | **23.896 ms median** | Same, no preview inputs |
| Owner-confirmed minimized counter window, no further input | **20.712 ms** | Worker publication matched to the app event |

The early **0.701 ms** hidden figure was the receiver's queue wait, not the complete publication-to-apply interval. The table uses the exact worker timestamp. [Corrected hidden measurement](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/hidden-summary.json).

**Minimize coverage is qualified:** automated GPUI and direct AppKit calls did not establish `NSWindow.isMiniaturized` under either Cargo or Blaze. The owner then confirmed minimizing the counter. WindowServer showed its native window as a **145×174 thumbnail**, while AppKit still reported a visible, non-miniaturized window with cached 1536×996 bounds. Delivery was verified in that owner-confirmed state, without further input. A strict **AppKit `isMiniaturized == true`** case is not established by this run. No engine or GPUI patch was made for it. [Owner-minimized event and WindowServer observations](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/minimized-check.json), [Cargo control](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/cargo-minimize-check.json).

The minimized delivery probe's manual CLI build was interrupted after a profile-setting mismatch; the daemon published the captured patch while that manual probe was still in progress. Its compiler wall time is excluded from performance comparisons. A subsequent replay addressed a different worker after multiple profiles were loaded and produced no event in the older process; it is retained as a discarded setup attempt, not a second passing minimized trial. Debugger experiments also interrupted one supervisor's child ownership (`ECHILD`), so they are not used as production acceptance results. All temporary native-window hooks are removed.

Idle CPU uses three paired **10-second** `proc_pid_rusage` samples, same executable with `RAKE_LIVE` absent/present, both windows hidden, with no harness inspector polling during each sample:

| Mode | Median CPU, percent of one core |
|---|---:|
| Polling off | **0.0026%** |
| UI timer on | **0.0705%** |
| Difference of medians | **0.0679 percentage points** |

The enabled app remains under the normal supervisor, including its runtime diagnostic polling; the off control is launched directly. These are app-process idle CPU measurements in the actual development setup, not an isolated timer microbenchmark. They exclude supervisor/compiler CPU, and the difference also includes normal diagnostic traffic.

[Raw paired samples](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/idle-cpu.json). The historical frame-poll sample was 0.1% off / 1.1% on from one ten-second pair; sampling precision and visibility differ, so no strict speedup ratio is claimed.

## Six passive text/helper trials

The actual command is `rake dev --compiler blaze --manifest-path examples/view/Cargo.toml --bin rake-view --json --no-open`. The app is behind the editor; **all 12 before/after preview records report occluded**. Setup clicks establish counter **2**. After each source save, the harness sends only diagnostic/tree queries: **no preview `Move`, click, key, redraw command or recorded-frame lease**. Redraw means the first completed inspected render containing the new marker, not physical display scan-out. The app itself requests the redraw after `Applied`.

| Six-trial median | Text apply | Text redraw | Helper apply | Helper redraw |
|---|---:|---:|---:|---:|
| Previous frame poll + preview-driven redraws | 0.945 s | 0.984 s | 0.477 s | 0.514 s |
| UI timer, passive observation | **0.830 s** | **0.897 s** | **0.401 s** | **0.621 s** |

PID **3521** and counter **2** survive all twelve steady trials. Apply improves in both cases; helper redraw is slower than the synthetic-input baseline. These protocols differ, so this is not an equal-condition compiler benchmark. Both redraw medians remain above the earlier 0.692/0.427 s upper targets.

**First-edit outlier:** after rebuilding the framework, the first timed attempt caused a full object rewrite (**22.8 s worker edit-to-binary**) and supervised relink/restart, rather than `Applied`. This is recorded separately; the standalone counter has no registered checkpointed state and starts at zero after replacement. Explicit warm-ups then took **1.301 s text / 0.608 s helper**, same PID. The table contains the following six edits of each kind; the outlier is not silently described as a fast warm patch.

[All steady trials](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/view-trials.jsonl), [derived medians](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/view-summary.derived.json), [warm-ups](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/warmup.jsonl), [supervisor JSON events](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/view-redraw-events.jsonl).

### Phase split

| Median stage | Text | Helper |
|---|---:|---:|
| Save → engine edit request, including supervisor detection/settle/dispatch | **220.1 ms** | **196.1 ms** |
| Engine settle | 1.654 ms | 1.652 ms |
| Source read | 0.745 ms | 0.030 ms |
| Source apply/outlines | **59.356 ms** | **4.708 ms** |
| Root work | 44.080 ms | 42.678 ms |
| Cached-code revalidation | **452.094 ms** | **107.227 ms** |
| Compile-rest bucket | 2.465 ms | 2.043 ms |
| Object | 0.981 ms | 0.998 ms |
| Link/splice stage | 1.093 ms | 1.119 ms |
| Whole worker edit → binary | **590.3 ms** | **184.9 ms** |
| Publication → app record | 25.539 ms | 23.896 ms |
| App record → observed redraw, median of per-edit gaps | **37.676 ms** | **227.559 ms** |

Individual stage medians do not sum to the median total. Other worker setup/static/reach/resolve/report buckets remain in the raw timelines. No CLI `blaze build` process starts for these successful normal edits; they use the resident daemon. Validation dominates the engine time. The longer helper post-apply rendering/capture gap is measured but not further profiled here; it is not counted as macro or compiler time. The engine is unchanged.

## What the old `entry-9` trace establishes

The old trace contains **`op-3` submitted/committed**, preparation and mounting of **new `entry-9` / `tab-8`**, and cancellation of older activity runs. The router allocates a fresh entry ID through navigation planning, independently of rendering. A metadata/view cache miss alone does not allocate a fresh logical entry. The trace does **not** retain the action, caller, target window or the old entry's checkpoint. Cancelled activity runs alone do not prove the entry's draft state was destroyed.

| Identity risk checked | Finding |
|---|---|
| Function/closure addresses | Callbacks are stored and invoked, but no address comparison/hash keys router entries. Entries, tabs and hosts use explicit numeric IDs. |
| Closure `TypeId` | No closure-type identity key found in Rake routing. Service/event registries use the concrete service/event type. GPUI `AnyView` identity is its entity handle; `AnyView` equality compares entities. |
| Vtable pointers | Trait objects provide dispatch; their vtable addresses are not compared as router/view keys. |
| Static addresses | Inspection's authored-definition memo uses pointer equality to recapture metadata. It does not create router entries. |
| Other pointer comparison | Router navigation-cache trimming compares two record addresses to avoid selecting the same record; it is not a route/view identity key or a mount trigger. |

Source owners: `router/runtime.rs` (`Entry`, `State::next`, `fresh`, navigation planning), `router/view.rs` (`with_views`/retained `AnyView`), `services.rs`, `tasks/runtime.rs`, `inspection/view/cache.rs`, and pinned GPUI `view.rs` (`AnyView::entity_id`, `PartialEq`).

**Smaller real reproduction:** a helper called by `AppRoutes::mount`, the registered checkpoint callback, and render was patched **32 consecutive times**. PID, runtime, window/tab/entry IDs and draft count **1** remain unchanged. No preview inputs are sent during the edits. [Initial identity and snapshot](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/small-initial.json), [32 rows](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/small-trials.jsonl).

**Inspection ambiguity reproduced:** after the fill/reopen, an explicit new-window command creates a second draft. Unqualified `inspect_tree` reports **Draft edits: 0** from the new view. Window-pinned inspection simultaneously proves the original **window 4 / entry 6 still has Draft edits: 1**. The captured controlled command is `op-3`; its new IDs are window 8 / tab 9 / entry 10, so it is not claimed as an exact replay of the older IDs. [Pinned control experiment](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/new-window-control.json).

The earlier harness checked unqualified text only. Its statement that state was lost at edit 30 was therefore stronger than the evidence. **The old operation's initiating caller remains unknown**, and there is no proven Rake address-identity bug or proven Blaze cause to patch. No speculative identity change was made. The new fill checks both state and complete logical identity after every edit, so a new view cannot silently be counted as the retained draft.

## Uninterrupted fill and zero-window lifecycle

The routing fixture already registers `DraftState::on_reload`. It calls both edited helpers from mount and checkpoint capture. A fresh process starts the default **256 MiB** patch area. After one setup click, edits 1–64 each change a marker and an approximately **4 MiB** literal; **no reseeding, reset, synthetic input or measurement restart occurs during the sequence**.

- **64/64** edits retain draft count **1** and window/tab/entry identity. Edits 1–63 keep PID **40027**.
- Edit **64** exhausts the actual room and requests `the room the program set aside for edits is used up`. The supervisor builds and checkpoint-restarts **40027 → 22082**, restoring the same logical IDs and count **1**. The runtime ID changes, as expected for a checkpointed restart. Save → redraw is **4.925 s**.
- A further body patch keeps PID **22082**, identity and state, in **2.973 s**. It still has the large literal fixture; this is not a small-edit median.
- Closing all windows reaches zero windows with the process alive. A new window then receives another live patch without preview-driven redraws. This is a deliberate disposal/recreation of a view, so its fresh draft is not reported as retained window-local state.

[Every fill snapshot and outcome](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/fill-trials.jsonl), [post-restart patch](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/fill-followup.json), [reopen proof](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/reopen.json), [routing supervisor JSON events](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/routing-fill-events.jsonl).

## Regression and history

All **13 final checks pass**. [Commands, durations and earlier attempts](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/regressions.json).

| Check | Final result |
|---|---|
| `cargo test --workspace --locked` | **925 passed, 4 ignored** |
| CLI package tests | **315 passed, 2 ignored**; included in workspace total |
| Live-runtime tests with `live,view-test-support` | **5 passed**, including the timer/fake-source regression |
| `scripts/test-supervisor.py` | **14 passed** |
| Preview / Web deployment Python tests | **18 / 10 passed** |
| Workspace and live-feature Clippy, all targets, `-D warnings` | Passed |
| Web default and optional-live target checks | Passed |
| Live library rustdoc | Passed |
| `rake format --check` / `rake lint` | Passed; **712 Rust files, 14 diagnostics, zero denied** |
| Plain Cargo `rake dev`, restored example | Window renders; click and key reach **Count: 2**; no live events |

First-pass failures and retries are preserved. New main's bundle test fixture omitted `Project::target_directory`; a one-line test-only initialization fixes its compile error. The timer test's unnecessary `mut` was removed. The shared macOS frame helper is gated for `live` or `devtools`, keeping ordinary builds free of unused-code warnings. These verification fixes do not change the measured live/devtools path.

One later workspace run failed main's unchanged `static_serving_keeps_mime_head_cache_redirects_and_html_fallback` test when its client received no HTTP headers. Five isolated retries and the final full workspace retry pass. No HTTP server fix or timeout relaxation was made; this intermittent result remains in `workspace-tests.serve-flake.log` and `serve-focused-retry.log`.

The original example files are restored byte for byte. Temporary minimize hooks and diagnostics are removed. Experiment apps, private daemon and the owned `caffeinate` process are stopped at completion. The original checkout, engine diff and all 13 protected hashes match their starting state. [Final integrity record](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/final-integrity.json).

Five local commits on `origin/main` retain the feature, original runtime polling, Rust supervisor, independent timer/redraw fix, and updated report, in that order. [Final commit list](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-evidence/timer/commit-list.txt). Nothing is pushed or posted upstream.
