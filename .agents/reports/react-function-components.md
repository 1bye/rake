# Function components: issue #27

Date: 2026-11-08. Branch: `feat/react-styled-function-component`. Base: `2289d0b`.
These results describe the working tree, not a published release.

**Automatic state-update isolation is fixed. Performance acceptance remains open.**
The feature stays experimental: parent renders, mount/replacement, and reorder workloads still exceed the approximately 10% overhead target against plain retained GPUI entities. Native leaf and bulk updates are substantially closer. Blaze live reload remains an external verification gap.

## Changes and ownership

`experimental-components` now selects the existing pinned gpui-fast 0.1.13 renderer on native and WASM. Default builds retain their existing backend. Consumers use `rake::gpui` for compatible types. The optional AppKit crate now receives the same backend selection; previously, enabling function components alongside native controls produced mismatched GPUI types. No dependency version pins changed.

A component entity owns shared hook storage. Registry entries hold weak references to both the entity and storage. Setters mutate the storage and notify the owning entity directly. They no longer lease the entity just to change a hook: that lease marked the entity's fields changed, causing ancestors that accessed it during mounting to rerender. The generation check and application identity still reject stale setters.

Props have separate shared storage. Equal memo props require no entity access; changed props update the owning entity so the retained renderer sees them immediately. Ordinary components intentionally rerun on a parent render, as before. `#[rake::component(memo)]` now reuses equal props without requiring explicit dimensions. Layout changes still propagate to siblings.

Effects and hook inspection execute inside the retained component boundary. Effect callbacks run outside the storage borrow; state-only hooks no longer reserve inline effect storage. Inactive inspector wrappers bypass capture bookkeeping. Context and loader observers notify their owner without leasing its fields.

Explicit IDs remain scoped by parent identity and concrete props type. Reordering preserves state; removal or identity changes dispose it. A real `uniform_list` regression verifies scrolling out and remounting with fresh state. GPUI's measurement row stays mounted even when offscreen. A retained page entity alone does not retain a removed tab subtree.

The original gpui-pre measurements and workaround-only isolation findings are superseded by this report. All comparisons below use gpui-fast for every variant; they must not be mixed with earlier backend results.

## Correctness and compatibility

The final code passed these checks:

- macOS ARM64: 139 library tests, 21 function-component consumer tests, and 31 existing view-state tests. One manual library benchmark remains ignored.
- Linux ARM64 container: the same 191 tests passed, including compile-fail consumer fixtures.
- Repeated local state updates preserve ancestor and sibling render counts; coverage includes macro-authored buttons and ordinary GPUI elements.
- Intrinsic-size memo reuse, changed props, layout reflow, after-paint effects and cleanup, stale setters, keyed identity, virtual-list disposal, context subscriptions, and resource cancellation passed.
- Focused Clippy passed with `-D warnings`, including native controls, examples, and the benchmark.
- WASM library and renamed-facade builds, native renamed dependencies, feature-disabled libraries, the default backend, and the native-controls example compiled.
- The AppKit integration target compiled with gpui-fast. Its real-window suite failed the same pre-existing toolbar accessibility assertion on both gpui-pre and gpui-fast on macOS 27.2. Full AppKit runtime acceptance is therefore not established.
- Rake formatting passed. Rake lint reported zero denied findings and the same 14 existing warnings. GPUI pin checks passed.

The router test host now explicitly notifies before drawing externally stored router state, matching production observation. An existing source-location test failed on both backends because it assumed a multiline class attribute began on the opening-tag line; it now checks each actual source line.

The manual workflow in `.github/workflows/function-components.yml` covers macOS and Ubuntu tests, Clippy, compatibility builds, quick lifetime benchmarks, and bounded native-window smoke tests. The Linux test, headless-benchmark, and window steps ran locally; the hosted workflow has not been dispatched.

## Measurement method

macOS 27.2 ARM64, Rust 1.98.0, optimized bench profile, gpui-fast 0.1.13. Each table entry is the median of three separate process results, with variant order alternated between rounds. Each process discards five warmups. Headless cases measure 31 samples; native cases measure 60. Cells show **p50 / p95 in milliseconds**, not pooled percentiles.

- `Plain`: ordinary retained entities with fields and setters.
- `Hooks`: ordinary function components, including normal rerenders when the parent renders.
- `Memo`: function components with equal-prop reuse, comparable to retained plain entities.
- `Raw`: explicitly keyed window state, without a component entity boundary.

All use the same visible row and one state value per row. Leaf/bulk mutation reaches row setters directly. The earlier native harness leased the host to retrieve setters, which unnecessarily invalidated that host; that artifact is fixed in both modes.

Headless timing includes mutation, render, layout, and paint through `TestAppContext`. Native timing begins inside render and ends at a final canvas paint; it excludes mutation, GPU submission/presentation, and display latency. Neither measurement is a GPU frame-time claim. These runs used a shared development machine; tail variance is visible and confidence intervals were not calculated.

## Headless results

All 144 process cases completed, covering six scenarios, two sizes, four variants, and three rounds. `mount` and `cycle` both replace the complete row set on every iteration in the current harness; they are repeated replacement measurements, not independent algorithms.

| Rows | Scenario | Plain | Hooks | Memo | Raw keyed |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1,000 | mount | 5.78 / 6.93 | 7.33 / 20.08 | 7.53 / 9.27 | 2.25 / 2.88 |
| 1,000 | parent | 1.11 / 1.57 | 4.08 / 5.97 | 1.48 / 2.12 | 1.64 / 2.04 |
| 1,000 | leaf | 1.12 / 1.57 | 0.55 / 1.29 | 0.54 / 1.07 | 1.69 / 2.23 |
| 1,000 | all | 6.04 / 9.33 | 5.75 / 7.02 | 5.77 / 7.46 | 2.25 / 3.25 |
| 1,000 | reorder | 3.76 / 5.88 | 5.22 / 7.50 | 4.82 / 6.60 | 1.66 / 2.18 |
| 1,000 | cycle | 5.85 / 7.17 | 7.42 / 8.53 | 7.39 / 8.80 | 2.46 / 3.11 |
| 10,000 | mount | 99.08 / 189.28 | 112.44 / 291.09 | 109.90 / 128.96 | 37.12 / 43.63 |
| 10,000 | parent | 20.03 / 31.23 | 74.58 / 291.48 | 32.00 / 91.39 | 24.74 / 60.94 |
| 10,000 | leaf | 26.03 / 38.44 | 15.48 / 21.62 | 13.35 / 60.85 | 27.27 / 39.07 |
| 10,000 | all | 123.60 / 193.45 | 116.78 / 225.55 | 117.98 / 159.16 | 34.25 / 53.36 |
| 10,000 | reorder | 60.62 / 88.93 | 68.45 / 81.05 | 73.90 / 86.99 | 25.55 / 26.93 |
| 10,000 | cycle | 87.80 / 124.94 | 112.22 / 133.97 | 116.99 / 155.05 | 37.82 / 69.88 |

## Native window results

All 72 planned parent/leaf/bulk cases produced a result. Two initial processes stalled waiting for AppKit frame delivery and reached the 45-second timeout: memo leaf at 10,000 rows and raw bulk at 10,000 rows. Each completed on retry; the successful retry fills that table cell. This intermittent stall remains unresolved and affects more than the function-component variant.

| Rows | Scenario | Plain | Hooks | Memo | Raw keyed |
| ---: | --- | ---: | ---: | ---: | ---: |
| 1,000 | parent | 1.14 / 1.36 | 3.63 / 4.48 | 1.57 / 2.37 | 1.86 / 2.52 |
| 1,000 | leaf | 0.70 / 1.26 | 0.71 / 1.11 | 0.74 / 1.17 | 2.03 / 2.68 |
| 1,000 | all | 5.86 / 7.04 | 6.17 / 7.38 | 6.21 / 7.59 | 2.91 / 3.78 |
| 10,000 | parent | 11.68 / 12.89 | 43.41 / 46.90 | 19.47 / 20.54 | 19.18 / 21.45 |
| 10,000 | leaf | 8.86 / 10.28 | 9.54 / 11.64 | 9.87 / 12.25 | 21.10 / 24.18 |
| 10,000 | all | 89.54 / 101.01 | 94.59 / 107.27 | 93.96 / 103.15 | 32.96 / 38.72 |

Ordinary hook p50 is about 1–8% above plain GPUI for leaf updates and 5–6% for bulk updates here. At 10,000 rows, memo leaf p50 is about 11% above plain and its p95 is about 19% above. These results do not satisfy an across-the-board 10% acceptance claim.

Parent renders still show substantial cost. Without memo, hooks rerun every child while plain entities reuse children; that semantic difference accounts for part of the gap. Even memo parent rendering is about 37–67% slower in native p50, so memoization alone does not close the performance gate.

## Allocation and lifetime evidence

Both platforms completed all 66 quick cases, including 1/10/50 hooks per component and async effect/resource completion and cancellation. Initial handles were disposed after unmount. Every async quick case ended with zero tracked live effect guards and zero tracked live resource futures.

Representative headless parent-render medians at 10,000 rows:

| Variant | Allocations per update | Live bytes above baseline after mount | Live bytes above baseline after unmount |
| --- | ---: | ---: | ---: |
| Plain | 10,068 | 144,345,013 | 116,557,733 |
| Hooks | 240,095 | 168,458,361 | 135,349,617 |
| Memo | 20,069 | 168,458,361 | 135,071,081 |
| Raw | 70,043 | 105,683,417 | 85,500,569 |

The allocator counts process-wide live allocations while the test window remains alive, including GPUI frame buffers, registry capacity, and retained layout storage. These numbers are not component-owned leak totals. Lifetime checks prove disposal of handles and the instrumented effects/futures; full heap attribution and exact retained entity/subscription counts remain uncollected.

A one-second macOS CPU sample of the 10,000-row memo parent case showed retained render-dependency copying, element identity/state lookup, retained prepaint/paint reuse, and layout work among the hot stacks. This supports further work on mount/identity and renderer bookkeeping. It does not assign precise percentages to hook storage or establish that every remaining cost belongs to Rake. The extra keyed mount boundary and prop comparison remain even when a memo body is skipped.

Linux validation used a Rust 1.98 Debian ARM64 container, three virtual CPUs, and an approximately 8 GiB Docker VM, with two build jobs and debug symbols disabled for development/test builds. Four Xvfb/Mesa native smoke cases each completed 60 measured bulk-update frames at 100 rows:

| Variant | Native p50 / p95, milliseconds |
| --- | ---: |
| Plain | 2.02 / 5.74 |
| Hooks | 1.43 / 3.80 |
| Memo | 1.81 / 14.43 |
| Raw keyed | 0.94 / 2.04 |

These VM smoke runs verify execution; they are not controlled Linux performance acceptance or physical-display/hardware-GPU proof.

## Remaining gates

1. Parent, mount/replacement, reorder, and tail-latency regressions need further optimization or an explicitly revised acceptance target. Automatic state-update isolation is no longer the blocker.
2. Native async/effect measurements, the complete native scenario matrix, controlled Linux measurements, heap attribution, and instruction counts remain incomplete. `xctrace` requires configured Xcode instead of the active CommandLineTools installation; Linux `perf` was unavailable. WASM compilation is not browser/GPU execution proof.
3. AppKit toolbar accessibility and intermittent native benchmark stalls need separate investigation. Neither is specific to the hook variant in the observed runs.
4. Blaze 0.20261027.0 (`49ca152`), installed in temporary storage, failed a fresh component-example build in libc 0.2.191: the build script requests `rustc --version`, which that Blaze release cannot answer. No reload or state-preservation claim follows. Upstream's current [v2 usage documentation](https://github.com/indexable-inc/blaze/blob/main/docs/usage.md#change-a-running-program) also states that the daemon does not yet send live patches; editing requires restarting. Rake cannot implement missing compiler patch delivery.

Raw benchmark output remains outside the repository under temporary storage. No JSONL report artifacts are added.
