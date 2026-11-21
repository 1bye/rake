# Rake on Rune evaluation

Two independent, unpublished Cargo workspaces run the same three-route app. The
baseline uses `rake::App`, Rake's typed router and layout, `view!`, GPUI Base's
text input, and GPUI's virtual uniform list. The Rune app owns its layout and
navigation state and uses Rune's native window, text input, virtual rows and
retargetable spring.

## Run

From this directory on macOS:

```sh
(cd gpui-baseline && cargo run --locked --release)
(cd rune-app && cargo run --locked --release)
```

Overview has a bar that can reverse before reaching its target. Activity shows
1,200 fixed-height events with stable IDs and mixed-length text. Settings has
an editable workspace name, a notifications switch and a light/dark toggle.
The controls and theme are in-memory preferences for the lifetime of the window.

The first Cargo run needs access to crates.io and the pinned Git revisions.
The GPUI baseline selects Rust 1.98.0 and pins Rake with `rake::motion` and
its GPUI Pre, Base and Web patches to
[`0274048d`](https://github.com/1bye/rake/tree/0274048dc500a747ebf178700bf920beca865dc8).
Rune selects Rust 1.99.1 and pins Rune and its five patches to
[`78efd4a7876b5478dd72033ce6fd2b2f13ab9f9a`](https://github.com/anomalyco/rune/tree/78efd4a7876b5478dd72033ce6fd2b2f13ab9f9a).
Cargo resolves overrides at each consumer workspace root.

Rune loads `fixtures/rake-installation.json` and resolves
`activity-samples.tsv` through `rake_resources::Resources`. The GPUI baseline
embeds the same text for consistent inputs. Set `RAKE_RUNE_RESOURCES` to point
the Rune executable at a relocated copy of the fixture directory. Set
`RAKE_RUNE_DIAGNOSTICS=1` to print bounded diagnostic records for navigation.
The records use stable `rake_protocol::router::RouteAddress` keys and
`rake_diagnostics::Collector` sequence numbers and timestamps. The workload
is entirely local.

## What Rune needed to replace

| Rake integration exercised by the baseline | Rune implementation |
| --- | --- |
| `rake::App` native bootstrap and window options | Rune's `App`, `AppSpec`, chrome and window loop. The experiment explicitly opens the resource directory and creates its diagnostic collector before handing off state. |
| Derived route registry, layout, route outlet and navigation driver | A three-variant route model with a selected route, stable keys, idempotent switching, and a direct view match. It emits the existing owned protocol address. |
| Rake `view!` markup, Base components and GPUI entity callbacks | Rune element builders and queued state handlers. Focus, keyboard, button roles and switch state use Rune's own control APIs. |
| Ambient Rake semantic palette and Base input/theme state | Rune design tokens, explicit theme selection and `TextEdit` bindings. |
| GPUI list windowing and frame scheduler | Rune growth stack and keyed virtual rows. Rune animates the bar's width with a property spring; GPUI translates a clipped, fixed-width bar with a presentation spring. Both carry velocity across reversals. |
| GPUI profiler and native window observation | Rune's frame hook for frame-build work. The external macOS harness drives the same workloads and observes the same process counters without either stack's frame hook. |

The prototype has three destinations in one window. Rake's route history,
multi-window and tab lifecycle, admissions and guards, task supervision,
services, crash reports, renderer inspection/control, packaging, updates and
Web/Wasm integration would each need separate evaluation before offering a
Rune-backed Rake product. This experiment exercises the portable resource,
protocol and diagnostic crates directly.

## Results with `rake::motion`

Both apps ran on the same Apple M4 Pro virtual Mac (12 vCPUs, 16 GiB,
macOS 26.6.2, 1× desktop, paravirtual Metal). These results characterize
this VM and workload, not physical Mac hardware; background services
were active. [Raw samples](artifacts/m4-pro-virtual-measurements.csv)

| Measurement | GPUI / Rake motion | Rune | Samples per stack |
| --- | ---: | ---: | --- |
| Clean dev build | 36.43 s | 31.65 s | 1 empty target |
| Warm dev rebuild after one-line view edit | 1.85 s | 1.52 s | 1 edit |
| Release executable size | 22,467,344 B | 27,156,288 B | 1 unstripped Mach-O each |
| Startup to on-screen window, median (range) | 137 ms (130–240) | 308 ms (295–343) | 5 launches |
| Idle CPU, median | 0.0948% | 0.0186% | 5 × 5 s |
| Idle interrupt wakeups, median | 38 | 7 | 5 × 5 s |
| Idle physical footprint, median | 28.9 MiB | 23.3 MiB | 5 readings |
| Animation Metal encoding per frame, median / p95 | 0.043 / 0.117 ms | 0.041 / 0.093 ms | 314 / 316 frames |
| Scroll Metal encoding per frame, median / p95 | 0.042 / 0.073 ms | 0.041 / 0.069 ms | 374 / 186 frames |
| Animation process CPU, median per run | 108.2 ms | 163.0 ms | 3 runs |
| Scroll process CPU, median per run | 228.7 ms | 119.2 ms | 3 runs |
| Animation present-request gap, median / p95 | 16.7 / 19.5 ms | 16.7 / 19.6 ms | 3 runs |
| Scroll present-request gap, median / p95 | 16.6 / 18.5 ms | 27.4 / 39.6 ms | 3 runs |

The original frame-build hooks measure different parts of each stack. This
run replaces those rows with the same external Metal encoding boundary on
both apps and separately reports process CPU over each whole workload. A
present request is not proof of display presentation; GPU execution and
display latency are not measured. Spring settle behavior and list overscan
still differ.

**Rune did not show a meaningful overall performance lead:** animation
encoding and cadence were close, and Rune used less CPU during scrolling
while requesting about half as many frames. Rune built faster in the
single build samples and used fewer idle resources; GPUI started faster
and produced a smaller release binary.

## Historical results (GPUI view-sampled tween)

Measured on an arm64 MacBook Pro (MacBookPro18,2, Apple M1 Max, 32 GiB),
macOS 26.6.2 (25G83), on AC power. Other processes were active during the
evaluation. The runtime values describe this host and this app workload.

| Measurement | GPUI / Rake | Rune | Samples per stack |
| --- | ---: | ---: | --- |
| Clean dev build | 182.95 s | 121.57 s | 1 empty target |
| Warm dev rebuild after one-line view edit | 3.07 s | 3.80 s | 1 edit |
| Release executable size | 22,242,224 B | 27,175,280 B | 1 unstripped Mach-O each |
| Startup to on-screen window, median (range) | 496 ms (438–647) | 639 ms (485–807) | 5 launches |
| Idle CPU, median | 0.0504% | 0.0141% | 5 × 5 s |
| Idle interrupt wakeups, median | 26 | 5 | 5 × 5 s |
| Idle physical footprint, median | 48.4 MiB | 44.1 MiB | 5 readings |
| Animation frame build, median / p95 | 0.374 / 0.509 ms | 0.065 / 0.168 ms | 3 runs, 560 GPUI and 624 Rune frames |
| Scroll frame build, median / p95 | 0.594 / 0.663 ms | 0.173 / 0.446 ms | 3 runs, 243 GPUI and 258 Rune frames |

The two frame hooks have different work boundaries. Treat the frame rows as
directional observations. GPU execution and display presentation cost are
unmeasured. The Rune spring preserves velocity when retargeted, while the
baseline smoothstep tween preserves position. Rune can draw additional settle
frames and uses a different list overscan strategy.

The clean builds ran sequentially with fetched dependencies. The samples
capture one pass through each dependency graph under background load.
The startup ranges overlap. [Raw samples](artifacts/measurements.csv)

| Route | GPUI / Rake | Rune |
| --- | --- | --- |
| Overview | [PNG](artifacts/gpui-overview.png) | [PNG](artifacts/rune-overview.png) |
| Activity | [PNG](artifacts/gpui-activity.png) | [PNG](artifacts/rune-activity.png) |
| Settings | [PNG](artifacts/gpui-settings.png) | [PNG](artifacts/rune-settings.png) |

All six PNGs are from native windows with a 960 × 640 point client area at
2× scale, cropped to 1,920 × 1,280 pixels. In the Activity screenshots both
stacks display the accented Latin, Greek, Japanese and Arabic fixture text.
The Rune row occupies more horizontal space. The screenshots include each
stack's default font, fallback, theme and line-height choices, so the visual
comparison covers each complete text stack. In the fourth Activity row, the
horizontal extent of pixels darker than 128 on an 8-bit grayscale conversion
is 821 pixels in Rune and 789 pixels in the GPUI baseline.

Both apps navigated through the sidebar and scrolled Activity during native
input checks. Editing the field to “Studio West”, switching notifications
off and enabling dark appearance survived leaving and returning to Settings.

**Wait on a Rake integration.** A small standalone Rune app
can reuse the three portable crates. It built faster from an empty target
directory and used fewer interrupt wakeups at idle on this host. The
prototype still needed a separate UI and routing implementation, the
unstripped binary was larger, and the consumer is tied to a pre-release
revision, five Git patches and another Rust toolchain. Product-level routing,
lifecycle and platform parity need their own evidence before committing to
that integration.

## Reproduce the measurements

Compile the macOS harness from this directory with
`swiftc -O window-check.swift -o /tmp/rake-rune-window-check`. Runtime
measurements need an interactive macOS desktop. The `capture` mode specifically
needs 2× scaling and Screen Recording permission. Synthetic input needs
Accessibility permission.

```sh
/tmp/rake-rune-window-check capture gpui "$PWD/gpui-baseline/target/release/rake-gpui-eval" "$PWD/artifacts"
/tmp/rake-rune-window-check capture rune "$PWD/rune-app/target/release/rake-rune-eval" "$PWD/artifacts"
```

Use `startup-idle` in place of `capture` and a scratch output directory for
five process launches per app. Use each default release executable, built
with `cargo build --release --locked --offline`. Measure its byte length with
`stat -f %z`. The startup timestamp is the first on-screen main window from
the launched PID reported by WindowServer, sampled with a 2 ms sleep between
queries. It is a window registration proxy for startup. Idle uses
`proc_pid_rusage` deltas over five seconds after a two-second settle. The
harness converts CPU ticks with the Mach timebase and reports CPU percent
relative to one logical CPU. Wakeups are Darwin's per-process interrupt
wakeups and memory is physical footprint.

For the same-boundary workload measurements, run the harness in `workload`
mode with both default release executables while recording Metal System Trace
(`xctrace record --template 'Metal System Trace' --all-processes`). Use the
harness's PID and epoch start/end times to select each scenario in the trace.
Use the `ca-client-present-request` and `metal-application-intervals` tables:
pool gaps between adjacent present requests within each run, and sum Metal
`Encoding` interval durations per frame for frames with a present request
in the scenario. Take the median and nearest-rank p95 across three runs.
The harness reports total process user + system CPU time in each interval.
It sends 12 alternating clicks 100 ms apart, then 80 pixel-wheel events of
34 points 16 ms apart on Activity. Discard runs that lose focus.

For historical in-process frame profiles, first build the GPUI app with
`cargo build --release --locked --features frame-profile`. Run the harness
with `frames` in place of `capture` and a scratch output directory. Each
trace has the frame's epoch timestamp and frame-build wall milliseconds.
Select samples inside the printed scenario intervals from the matching run's
log. The historical table pools
frames across the three runs and uses the nearest rank for p95. Rune's frame
update includes runtime event work after drawable acquisition. GPUI's
`Window::draw` measures its view/layout/paint and scene work. The two hooks
provide different scopes of CPU-side frame work.

For the Cargo timings, run from each app directory. Prefetch dependencies
first. Use an empty target directory, the default dev profile with debug info
disabled, and the checked-in lockfile.

```sh
mkdir -p target
export CARGO_TARGET_DIR="$(mktemp -d "$PWD/target/bench-dev.XXXXXXXX")"
export CARGO_PROFILE_DEV_DEBUG=0
/usr/bin/time -p cargo build --locked --offline
```

For the warm build, replace `"Workspace preferences"` with
`"Workspace settings"` in `src/main.rs` and repeat the same timed command
using the same target directory. Restore the view text afterward.

## Verification

Run these from each nested workspace:

```sh
cargo fmt --all --check
cargo test --release --locked
```

Both workspaces passed four release tests and `cargo fmt --all --check`.
The original evaluation also ran Clippy with
`--all-targets --no-deps -- -D warnings`. That GPUI Clippy run
also enabled the frame profiling feature.
Rake's Cargo workspace commands use its own member list. I ran the Rake
source formatter and source lint for both experiment directories, and
checked the shared `activity.rs` explicitly. Those source commands can
operate on the nested workspaces. The Rake CLI was
`$HOME/.cargo/bin/rake` to select it over Ruby's executable of the same
name. Lint reported zero findings. Deep Rake lints, Windows, Linux, Web/Wasm,
VoiceOver, and packaged distribution were outside this run.
