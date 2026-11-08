# Published Blaze app trial

The published bundle builds 12 of the 13 examples and starts 11. It also builds and runs the external common-dependency app. Body patches preserve tested state. Automatic field-change restart is **not generally safe without registered checkpoints**: Notes passes, Kit loses its unregistered count, and standalone-root apps need the new restart guard. GPU-surface is blocked by a larger engine constructor-selection problem; Inspector is blocked by its existing manifest.

## Baseline and method

- Branch `codex/blaze-live-app-trial`, based on the owner's explicit `85500e9babec66d1dce1d8cf11d254d69a04f840`. The locally named supervisor branch pointed at `ee0ec974`; the requested PR SHA was used.
- Published `1bye/blaze` `v0.20261030.1-rake.1`; SHA-256 `aec30a92722bed32f7dd5d101520ab5554149339a66ac670a352065865457e10`. The initial run moved the existing `~/.rake` aside, downloaded from GitHub with no Blaze on PATH, and cleared `RAKE_BLAZE` and `BLAZE_ARCHIVE`. `rake doctor` reports ready, matching checksum and macro server. Previous installation preserved in the evidence directory.
- [Raw evidence, logs, screenshots and reproduction scripts](/Users/yuriihulyk/Documents/Codex/2026-11-08/blaze-live-app-trial-evidence/). Scratch source is outside the repository at `/Users/yuriihulyk/Documents/GitHub/rake-scratch/blaze-live-app-trial`.
- Empty Blaze key store and separate target per cold app; warm rechecks restore that app's store. Cargo baselines share a Cargo target and are not cold-time comparisons. Three Cargo jobs, 64 MiB stack, native macros, sandbox on, `--bin`, line-table debug information, existing live-mode `pf-no-simd`. No engine fallback or engine modification.
- Times below are seconds. Cold column is **Blaze compiler / CLI start-to-watching**. First Agent additionally includes installation/toolchain setup: its engine reports 161.80 s, compiler process 218.11 s, whole CLI 326.79 s. Other CLI totals include metadata, validation and launch; they are not compiler-only times.
- Edit columns are **save → apply / save → published redraw tree**, medians for Notes/View (three trials each), one trial elsewhere. Helpers were introduced as temporary fixtures before timed body edits. Routing and tray originally use raw GPUI builders: a temporary `view!` label and state witness supplied the requested macro edit. No fixture remains in the committed example source.
- Timed edits use app diagnostics and inspection trees, without preview leases or input-driven redraws. This is not display scanout. Input is used outside timings for feature/setup checks. The Mac initially locked, then the owner unlocked it. Black/off-screen captures and failed harness waits remain in evidence. Stage Manager/Space and foreground changes prevented a controlled behind-editor comparison throughout; occlusion and foreground observations are recorded per edit. Distribution's recheck was foreground. Do not compare these as controlled PR #53 latency benchmarks.
- Every compiler-invoked example and the scratch build had **zero refused functions and zero macro expansion failures**. Inspector never reached the compiler. The `bad-toml/Cargo.toml version = 1` warning comes from a deliberately invalid dependency test fixture and appears under Cargo too; it is not a failed expansion.

## App matrix

| App | Builds / macro failures | Cold compiler / CLI | Starts | Text apply / redraw | Helper apply / redraw | Field change | State / platform / notes |
|---|---|---:|---|---:|---:|---|---|
| agent | Yes / 0 | 161.80 / 326.79 | Yes | 2.417 / 2.496 | 0.423 / 0.469 | Blocked; same PID | Count 2 kept; root restart blocked |
| desktop | Yes / 0 | 157.83 / 184.93 | Yes | 2.428 / 2.481 | 0.467 / 0.499 | Blocked; same PID | Dirty Home kept; MissingState blocks restart |
| distribution | Yes / 0 | 151.90 / 178.23 | Yes | 0.730 / 0.775 | 0.401 / 0.423 | Blocked; same PID | Local control opt-in; root restart blocked |
| files | Yes / 0 | 152.05 / 178.80 | Yes | 1.865 / 1.893 | 0.279 / 0.305 | Blocked; same PID | Import/normalize after patch; root restart blocked after safety fix |
| foundation | Yes / 0 | 205.47 / 233.33 | Yes | 2.448 / 2.503 | 0.865 / 0.892 | Restart 4.213 s; before root guard | Local control opt-in; bundle/update metadata differs |
| gpu-surface | Yes / 0 | 231.47 / 258.90 | No | — | — | — | --features fast; runtime abort from duplicate GPUI class registration |
| inspector | No / — | — / — | No | — | — | — | Excluded/deprecated manifest fails before either compiler |
| kit | Yes / 0 | 199.48 / 229.25 | Yes | 1.792 / 1.950 | 0.421 / 0.563 | Restart 4.373 s; count lost | --features kit; controls work, but unregistered count resets on restart |
| notes | Yes / 0 | 268.29 / 297.27 | Yes | 3.484 / 3.586 | 0.571 / 0.723 | Restart 5.752 s | --features gui; unsaved library restored in all three restarts |
| preview | Yes / 0 | 301.14 / 329.08 | Yes | 2.855 / 3.027 | 0.455 / 0.533 | Blocked; same PID | Blur/opaque toggle works; root restart blocked |
| routing | Yes / 0 | 289.64 / 319.44 | Yes | 0.573 / 0.609 | 0.455 / 0.488 | Restart 3.855 s | 32 consecutive helper patches retain PID, route IDs and draft |
| tray | Yes / 0 | 227.73 / 256.15 | Yes | 0.558 / 0.615 | 0.473 / 0.550 | Blocked; same PID | Local control opt-in; native probe passes after patch; root restart blocked |
| view | Yes / 0 | 177.95 / 207.13 | Yes | 0.905 / 0.944 | 0.533 / 0.575 | Blocked; same PID | Three trials per edit; Count 2 retained; root restart blocked |
| scratch | Yes / 0 | 202.59 / 238.89 | Yes | 3.011 / 3.031 | 0.316 / 0.359 | Restart 3.233 s | All dependency outputs match Cargo; Count 2 checkpoint restored |

## Refusals, differences and Rake fixes

### GPU-surface: larger engine problem, not patched

The original `rake-gpu-surface --features fast` compiles, then aborts before opening a window:

```text
thread 'main' panicked at .../gpui-fast-macos-0.1.13/src/window.rs:415:60:
called `Option::unwrap()` on a `None` value
```

The failing expression is `ClassBuilder::new(name, superclass).unwrap()` for `GPUIWindow` / `NSWindow`. Mach-O constructor records prove Blaze retains both CE and Fast backend window/platform constructors (four records), while Cargo retains one backend's pair (two records). The hashes are CE `48d75cb2341937ca`, Fast `33888295d8d3bf24`; the Fast hash matches the panic stack. Both implementations register `GPUIWindow`, `GPUIPanel` and `GPUIView`. A Cargo comparison with matching `fast,rake/live,rake/devtools` still retains one pair and opens successfully, so enabling live features alone does not explain the failure.

Reproduction: the matrix command in `cases/gpu-surface/blaze/launch.json`. Evidence: `constructor-registration.json`, `constructor-registration-matched-features.json`, the runtime backtrace and Cargo screenshot under that case. Suggested fix: align Blaze's linked-crate/constructor-root selection with rustc, retaining only the selected backend. This is larger than an intrinsic lowering and was left open. No fork commit, no engine changes, no upstream post.

### Inspector: existing manifest failure

Both plain dev and live mode fail before compilation:

```text
error: failed to parse manifest at `/Users/yuriihulyk/Documents/GitHub/rake-worktrees/blaze-live-app-trial/examples/inspector/Cargo.toml`
error inheriting `edition` from workspace root manifest's `workspace.package.edition`
failed to find a workspace root
```

The deprecated example is excluded from the workspace but still inherits workspace fields. No platform/patch tests are possible. Suggested project follow-up: retire the example explicitly or make its standalone manifest complete; it was not migrated during this trial.

### Unsafe/unavailable checkpoints

Files imported the owner-selected `/private/tmp/rake-blaze-trial.txt` through the native picker **after** its text/helper patches. Normalization changed 19 bytes of CRLF input into 17 bytes of LF output. The source file remained unchanged. Before the new guard, adding `Toolkit`'s field restarted in **5.027 s** and lost the unsaved transformation. A quit guard alone does not serialize a standalone root's services.

Local Rake fix: Blaze restarts with no routed entries now fail with:

```text
Blaze restart blocked: standalone root state is not checkpointed; use checkpointed routed state, or explicitly opt into losing state with --restart-without-state
```

The final Files recheck blocked its field change in **1.025 s**, retained the same PID and unsaved 17-byte document, and kept the blocked status visible after two seconds with the rebuilt CLI. The old process stays alive. The explicit override still permits losing state, and the Cargo path is unchanged. Focused tests cover all three cases. Body patches remain enabled.

Desktop's dirty `Home` has no registered reload state. Its first field-change failure is `{"Reload":{"MissingState":2}}`; the app and draft remain alive. Before the fix, the supervisor retried that permanent failure until `"RequestCapacity"` replaced it after roughly 100 seconds. Local fix consumes that restart request, retries only Busy, and accepts a later edit's distinct restart request. Another real recheck exposed a stale `Applied` sequence clearing a blocked error; the permanent-error path now consumes that already-seen sequence too. Both regressions were demonstrated failing before their fixes and passing afterward. Distribution's blocked event arrived in 1.401 s; its old harness waited 150 s because the stale applied notification hid the transient status. The adjudication file preserves this distinction.

Kit's `Demo` field edit restarted in **4.373 s**, restored tabs, and reset the visible count from **2 to 0**. That field never registers reload state or marks its entry dirty. No checkpoint implementation was added to Kit just to make acceptance pass. Notes, routing and the scratch fixture exercise actual registered checkpoints. Notes created a fresh unsaved note before each of three restarts; the disk library did not exist before those restarts, and all notes survived. Its restart median is **5.752 s**. Temporary fixture edits were restored.

### Control endpoint and native target diagnostics

Distribution, foundation, GPU-surface and tray now opt into `.control(ControlConfig::FromEnvironment)`, the same environment-gated endpoint as View. As shipped, Distribution's text/helper changes visibly patched (OCR observation upper bounds 1.486 / 1.319 s), but a field edit did not restart and no apply event was observable. The final timed recheck includes this local example change. The other three examples' measurements also include the opt-in.

A library-only scratch package originally produced `select one binary with --bin, or a single cdylib for Web`. Local Rake diagnostics now name the package, explain that a cdylib alone is not a native app, and list binaries for a misspelled `--bin`. Real before/after CLI evidence and two integration tests cover this. Every listed example has a bin; those also producing cdylibs are run with `--bin`.

### Bundle-relative behavior

Foundation's layout and ordinary text match Cargo, but its update status differs: Cargo displays **“Updates disabled in this build”**, Blaze **“Updates unavailable in this build”**. The Cargo dev path makes an application bundle containing `Contents/Resources/rake-installation.json` with updates disabled; `blaze::build` launches the compiler's executable directly and has no installation descriptor. `Resources::discover` searches relative to that executable. This is a Rake live-mode packaging limitation, not evidence of a miscompiled enum. The descriptor, paths and source call path are saved in `foundation-bundle-difference.json`. Fix suggestion: stage dev resources/installation metadata adjacent to the live executable while retaining its live identity. Not implemented in this pass. Distribution's package/resource acceptance branch was not exercised; launch and hot patches do not prove packaged asset/update behavior.

Foundation's HTTP message endpoint was intentionally absent; both modes displayed the same transport failure. No updater installation, login-item change, production network action or user-file write was performed. The tray probe reports that drag/drop is unsupported by its current adapter in both modes.

## Common-dependency fixture

The external app uses this worktree's Rake by path and locks image 0.25.10, serde 1.0.229, serde_json 1.0.154, Tokio 1.54.1, reqwest 0.12.28, regex 1.13.1, chrono 0.4.45, uuid 1.29.0, rand 0.9.5, sha2 0.10.9, tracing 0.1.44, tracing-subscriber 0.3.23 and bundled rusqlite 0.37.0. `dependency-comparison.json` contains the full Cargo/Blaze values, exactly equal:

- PNG and JPEG each decode to 32×16 and have identical RGBA pixel hashes.
- serde derive/JSON parses 17; Tokio multi-thread task returns 41 and filesystem round-trip returns `async-file-ok`.
- reqwest verifies a disposable local HTTPS server with a fixture CA and parses JSON 23; no external service dependency.
- regex returns `hello`; chrono returns `2023-11-14T22:13:20+00:00`; UUID is v4; seeded rand returns 559256596868823998.
- SHA-256 of `abc` is `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad`; tracing captures its event; bundled SQLite sums 7+11 to 18.

The first actual cold compile took **202.59 s**, 188,700 functions / 404 crates; CLI start-to-watching **238.89 s**. Its phase summary reported codegen 69 s, host scripts/macros 54.2 s, names 34.9 s. A counter-setup harness timeout occurred before edits, then warm retry passed. Text **3.011 / 3.031 s**, helper **0.316 / 0.359 s**, same PID and count 2; a field change restarted in **3.233 s**, restoring count 2. These are single edit observations, not medians from six trials.

Two earlier attempts stopped during planning because the external app lockfile lacked the optional `blaze-live` Git pin. Adding a root `[features] live = ["rake/live"]` and running `cargo metadata` persisted exactly that one missing locked Git dependency; no existing locked version changed. Ad hoc feature metadata resolution did not persist it. This prerequisite is documented; no speculative CLI resolver change was retained. An early Cargo fixture setup used an older toolchain and two incorrect API calls; the fixture was corrected and compared on the repository's pinned 1.98 toolchain before assessing Blaze. No dependency/compiler fixes were needed.

## Platform checks and screenshot comparison

Cargo and Blaze captures are under each app's `cases/<app>/cargo*` and `blaze*` folders, with inspection trees, snapshot/state records and capture errors beside them. Agent, desktop, files, kit, notes, preview, routing, tray, view and scratch show matching layout/text aside from deliberate edits, state differences, window geometry, cursor/focus and async status. Routing’s initial periodic counters differed (Cargo 1/RunId, Blaze 0/None) under different visibility states; that snapshot alone does not establish a scheduler difference, and periodic-task parity was not separately established. Initial black captures were not counted as visual passes; usable later captures were compared where available. Foundation's update-status difference is recorded above. GPU-surface has only a healthy Cargo window. Inspector has no window.

- Files: native picker import works after patches; normalization produces the expected bytes. Both Cargo and Blaze completed normalization while the published tree still showed 19 bytes; an untimed redraw input updated it to 17 bytes. This reproduces the stale ordinary-task redraw under Cargo too, rather than establishing a Blaze-only paint bug. The earlier normalization harness timeout is retained; the final step-by-step recheck passed.
- Tray: the same temporary main-window fixture was used in both modes. The existing native probe passed after patches: native title/icon update, transient tray creation/removal, popup cycles, guarded quit, focus-existing without remount, close-all/reopen with the edited label and progress retained. Input-driven frame nudges were confined to that untimed native probe. Login-at-launch was queried, not toggled.
- Kit: required `kit` feature used; native opaque button clicks incremented the count, verified by screenshot/OCR rather than treating absent inspection labels as a failure.
- Preview: native backdrop blur/opaque toggle still works after helper patch.
- Routing: the 32-edit helper soak retains PID, tab/entry IDs and draft count 3 through every edit, including edit 30. It does not prove the cause of the historical remount is fixed. Explicit Open/Select commands opened a second document tab after patching. A `Screen` field change restarted in **3.855 s**, restored draft count 3, and a subsequent helper patch applied/redrew in **0.397 / 0.429 s** on the new PID. The first raw-click navigation harness timed out; the explicit command recheck passed and is retained separately.

## Regression, scope and verdict

The affected CLI suites pass: **229 unit tests (2 ignored), 16 project-build tests and 15 project-dev tests**. The Python supervisor suite passes **14 tests**, with `CARGO_TARGET_DIR` pointing at the actual trial CLI. The first Python invocation looked in the default target and failed to find the binary; the corrected invocation passed. Rake formatting and linting cover retained changes. Whole-workspace and Linux acceptance were not rerun in this task; these counts do not claim that coverage.

No engine/fork intrinsic patch was necessary: no unsupported intrinsic was reached with the requested published bundle and existing `pf-no-simd`. The fork remained unchanged, so fork commit is **N/A** for every finding. All eight protected binary hashes match the baseline (the old pinned installation is preserved separately). The original checkout HEAD and recorded dirty status also match. Fork HEAD remains `6625a86d591c106f92eba1dd62de8dd972c22baf`, branch `rake-patches`, clean. The release, toolchain and protected original binaries were not modified; nothing was pushed or posted.

**Verdict:** suitable for a narrowly documented experimental opt-in with these local safety/diagnostic fixes, not a claim that every Rake app supports state-preserving field edits. Docs must retain Apple Silicon/pinned-bundle restrictions, required bin/control endpoint, explicit checkpoint registration (including service data), root restart blocking/override, external app live-feature/lock prerequisite, unbundled resource/update differences and the unsupported Fast-backend example. Notes and the dependency fixture demonstrate use beyond View. GPU-surface and Inspector remain explicit exclusions. The historical router remount is not reproduced by this run, but its original cause remains unknown. Latencies are qualified by uncontrolled window placement and host load.
