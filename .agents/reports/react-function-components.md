# Function components: issue #27

Date: 2026-11-08. Branch: `feat/react-styled-function-component`.
Base: `2289d0b`. These results describe the working-tree implementation, not a published release.

The full timing and allocation tables below predate the simplification follow-up. They remain investigation data for the earlier implementation, not fresh measurements of the current refactor.

**Status: experimental implementation; not ready to close issue #27.** The core hooks, macro, identity, cleanup, resources, and inspector integration work. Automatic render isolation, performance acceptance, complete native/platform measurements, and Blaze reload proof remain open.

## Implementation and ownership

- `#[rake::component]` creates cloneable props and a hidden retained GPUI view. The existing `view!` and struct-update syntax remain intact; renamed dependencies are supported.
- `experimental-components` is opt-in. Ordinary props do not need `PartialEq`; the `memo` option requires it.
- One entity owns the hook vector. Call order selects slots. Count, kind, and storage-type changes are diagnosed; exchanging indistinguishable hooks cannot be detected.
- `State`, `StateRef`, and `Resource` are UI-thread `Copy` handles. An app-local registry stores weak owners and checked generations. Handles neither retain owners nor reach replacements.
- State snapshots are owned; functional updates read current storage. Refs do not notify. Memo dependencies compare by equality.
- Effects are deferred from paint, execute outside the entity borrow, clean up before replacement, and clean up on disposal. Resources use Rake managed loaders and cancel on replacement/unmount.
- Context hooks own subscriptions to existing GPUI globals; theme observation uses the existing theme global.
- Inspector metadata includes slot position, kind, type, and supported scalar/string previews. Limits: 64 slots, 256 UTF-8 bytes per string/type, 16 KiB per boundary; opaque values require no `Debug`. CDP exposes `rakeHooks`.

## Identity and rendering findings

The raw source-location-state fixture reproduces the sibling collision: two `RenderOnce` instances call `window.use_state` before their returned root IDs enter the element path, and receive the same entity. The benchmark raw baseline explicitly scopes keyed state with a stable row ID; it does not benchmark the broken collision case.

Function components mount under an explicit ID, parent element path, and concrete props type. Reordering preserves state. Insertion initializes only the new instance. Removal and remount reset it; old setters fail. Identical child IDs under different parents remain independent.

A retained page entity is not a retained element subtree. The tab fixture keeps both page entities alive but renders only the selected one, matching the current route outlet policy. After switching away, the removed component is disposed; switching back starts fresh. Cached mounted subtrees preserve element state. Virtualized rows follow the same rule when they leave the rendered tree; a real virtual-list integration is not yet exercised.

Pinned GPUI runs the root render on a draw and reruns uncached descendants. A leaf setter notifies only its owning entity, but this does **not** prevent uncached ancestor/sibling function execution. The regression fixture explicitly confirms this behavior. `memo` can reuse GPUI subtrees only with `.into_element().cached(style)` and definite dimensions; equal props alone cannot cache arbitrary `AnyElement` trees across frames. Guessing dimensions would alter layout. Automatic content-sized memoization/render isolation needs a GPUI-level design before merge.

## Measurement method

- Mac14,9, 10 logical CPUs, 16 GiB RAM; macOS 27.2 (26B5091g); Rust 1.98.0, aarch64-apple-darwin.
- Pinned `gpui-pre` 0.3.11, Zed revision `f816e1f9263e3a8ef63c4baaa02e1441ce1c6038`. `gpui-fast` 0.1.13 was compile-checked separately.
- Cargo release/bench optimization, normal release assertions policy, `experimental-components,view-test-support`; runtime shader compilation avoids requiring the Metal compiler for test consumers.
- Plain: a GPUI entity with ordinary integer fields held in a vector. Hooks: one generated entity with state slots. Raw: keyed GPUI entities inside `RenderOnce`. All render the same fixed-size text rows and perform the same updates.
- Headless timings include mutation plus synchronous CPU render/layout/paint. They exclude executor settling and GPU submission/presentation. Async cases additionally time the complete mount/settle/unmount cycle.
- 31 measured samples after five warmups for state scenarios; async scenarios use three warmup cycles. Sizes: 1, 100, 1,000, 10,000. Hook counts: 1, 10, 50 at 100 instances.
- `mount` and `cycle` both replace all keys with fresh keys each frame; they are repeated fresh-mount workloads, not independent cold-start measures. `reorder` reverses, removes one key, and inserts a new stable key.
- A counting system allocator records allocation calls and outstanding requested bytes. It includes framework, text/layout caches, and harness work. It is not RSS, heap attribution, or an instruction counter, and atomic instrumentation can affect timing.
- A/B/C run sequentially on a shared desktop without CPU pinning or randomized order. Large p95 outliers occur. These are investigation results, not statistical performance acceptance.

Timing columns below are microseconds, p50 / p95. Ratio compares hooks p50 to plain p50.

| Scenario | Instances | Hooks | Plain | Function | Raw keyed | B/A |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| mount | 1 | 1 | 10.8 / 12.1 | 11.5 / 11.8 | 10.6 / 10.8 | 1.06 |
| parent | 1 | 1 | 10.3 / 19.0 | 10.6 / 10.7 | 10.0 / 10.1 | 1.03 |
| leaf | 1 | 1 | 11.2 / 28.7 | 14.0 / 14.6 | 10.7 / 10.9 | 1.25 |
| all | 1 | 1 | 11.3 / 16.1 | 11.3 / 13.5 | 10.7 / 10.8 | 1.00 |
| reorder | 1 | 1 | 10.7 / 11.0 | 11.1 / 11.8 | 10.5 / 10.8 | 1.04 |
| cycle | 1 | 1 | 10.7 / 10.8 | 11.1 / 11.4 | 10.5 / 10.7 | 1.04 |
| mount | 100 | 1 | 657.9 / 1386.5 | 708.8 / 967.2 | 687.5 / 974.5 | 1.08 |
| parent | 100 | 1 | 658.5 / 1450.0 | 675.4 / 708.3 | 640.0 / 756.6 | 1.03 |
| leaf | 100 | 1 | 640.0 / 1513.8 | 676.0 / 709.9 | 637.8 / 804.0 | 1.06 |
| all | 100 | 1 | 658.5 / 876.1 | 690.6 / 709.1 | 654.9 / 719.4 | 1.05 |
| reorder | 100 | 1 | 631.6 / 697.7 | 677.5 / 683.2 | 656.8 / 839.7 | 1.07 |
| cycle | 100 | 1 | 660.5 / 915.8 | 702.2 / 709.7 | 730.0 / 1204.2 | 1.06 |
| mount | 1,000 | 1 | 8683.8 / 9247.2 | 9461.9 / 15628.7 | 10033.8 / 11155.5 | 1.09 |
| parent | 1,000 | 1 | 9651.5 / 14807.2 | 8545.0 / 10248.9 | 9732.7 / 15136.5 | 0.89 |
| leaf | 1,000 | 1 | 8524.8 / 9692.8 | 9294.7 / 10766.2 | 9312.8 / 10361.8 | 1.09 |
| all | 1,000 | 1 | 9140.9 / 10119.0 | 8979.1 / 10542.0 | 9812.9 / 19334.5 | 0.98 |
| reorder | 1,000 | 1 | 9562.0 / 12411.4 | 9427.0 / 10811.0 | 9745.1 / 11880.9 | 0.99 |
| cycle | 1,000 | 1 | 9276.9 / 19010.6 | 10378.1 / 11302.0 | 10042.2 / 11047.8 | 1.12 |
| mount | 10,000 | 1 | 91711.0 / 274810.8 | 104578.8 / 113245.8 | 88384.5 / 109840.5 | 1.14 |
| parent | 10,000 | 1 | 86765.5 / 91524.8 | 96803.2 / 103960.6 | 78253.9 / 102574.0 | 1.12 |
| leaf | 10,000 | 1 | 87548.7 / 94583.3 | 93497.1 / 101937.0 | 94464.0 / 119121.1 | 1.07 |
| all | 10,000 | 1 | 93047.3 / 119840.2 | 101781.6 / 109960.7 | 97976.1 / 148599.8 | 1.09 |
| reorder | 10,000 | 1 | 89987.5 / 117114.4 | 96401.8 / 105774.0 | 78365.4 / 86662.3 | 1.07 |
| cycle | 10,000 | 1 | 82364.0 / 88156.7 | 98988.9 / 106721.6 | 87375.6 / 94583.1 | 1.20 |
| hooks | 100 | 1 | 602.1 / 610.4 | 674.5 / 692.8 | 629.5 / 658.3 | 1.12 |
| hooks | 100 | 10 | 602.5 / 608.6 | 684.8 / 708.1 | 916.7 / 930.0 | 1.14 |
| hooks | 100 | 50 | 618.5 / 628.7 | 718.9 / 843.4 | 2172.0 / 2203.0 | 1.16 |
| effect_resource_async | 100 | mixed | 296.5 / 338.6 | 292.8 / 455.7 | 305.7 / 316.1 | 0.99 |
| effect_resource_cancel | 100 | mixed | 298.9 / 391.0 | 297.0 / 478.2 | 316.3 / 443.9 | 0.99 |

## Native macOS window matrix

All 81 state-workload cases completed in real GPUI windows with 60 CPU frame samples after five warmups each: all three variants, six scenarios at 1/100/1,000/10,000 instances, and 1/10/50 hooks at 100 instances. Each child process exited successfully. One initial 1,000-row plain mount attempt timed out; independent retries passed, and the resumed matrix completed without further failures. This is another reason not to treat the run as controlled acceptance data.

Timing starts at the native root render and ends in a final paint callback. It excludes the preceding mutation callback and GPU submission/presentation. Large trees paint only the visible portion of the window. These numbers are not directly interchangeable with headless mutation-plus-draw times. Native allocation/subscription counts and native async workloads have not been measured.

Selected results in microseconds, p50 / p95:

| Scenario | Instances | Hooks | Plain | Function | Raw keyed | B/A p50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| all | 100 | 1 | 528.3 / 878.9 | 559.0 / 1478.2 | 501.6 / 603.5 | 1.06 |
| all | 1,000 | 1 | 4323.6 / 4979.5 | 5062.4 / 5660.8 | 4426.3 / 4909.6 | 1.17 |
| mount | 10,000 | 1 | 34216.3 / 39168.0 | 45389.0 / 48194.9 | 41640.6 / 47364.2 | 1.33 |
| parent | 10,000 | 1 | 34953.0 / 39752.2 | 43942.9 / 48650.2 | 38641.1 / 42072.5 | 1.26 |
| leaf | 10,000 | 1 | 35783.0 / 38871.6 | 44279.5 / 48364.4 | 38719.0 / 43426.9 | 1.24 |
| all | 10,000 | 1 | 34196.3 / 38211.9 | 42914.2 / 48572.5 | 36727.5 / 42227.9 | 1.25 |
| reorder | 10,000 | 1 | 34570.5 / 39580.5 | 44630.6 / 49192.2 | 39412.7 / 43725.0 | 1.29 |
| cycle | 10,000 | 1 | 35018.0 / 38837.2 | 42981.8 / 49588.1 | 40910.5 / 48845.4 | 1.23 |
| hooks | 100 | 50 | 420.0 / 830.8 | 514.3 / 1068.7 | 1275.2 / 1862.6 | 1.22 |

At 1,000 instances, hooks p50 is 13–19% above plain GPUI across these six cases; at 10,000 it is 23–33% above. The earlier 100-row smoke run measured 14% overhead for all updates, while the full matrix measured 6% for that case. The variability does not erase the larger regressions. The performance gate remains open.

## Allocation and memory evidence

Representative one-hook workloads; bytes are deltas from an empty, already-created test window. Retained bytes are measured after clearing the tree, retiring frames, settling tasks, and dropping external probes. The application/window remains alive.

| Instances | Variant | Allocations per unchanged parent draw | Live after mount bytes | Retained after unmount bytes |
| --- | --- | ---: | ---: | ---: |
| 1 | Plain | 47 | 7,172 | 6,692 |
| 1 | Hooks | 49 | 8,788 | 7,748 |
| 1 | Raw | 49 | 8,940 | 7,852 |
| 100 | Plain | 2,647 | 768,108 | 750,404 |
| 100 | Hooks | 2,847 | 855,764 | 780,972 |
| 100 | Raw | 2,847 | 864,276 | 779,596 |
| 1,000 | Plain | 26,065 | 9,210,932 | 9,034,796 |
| 1,000 | Hooks | 28,065 | 11,191,964 | 10,443,924 |
| 1,000 | Raw | 28,065 | 10,194,140 | 9,346,612 |
| 10,000 | Plain | 260,087 | 87,410,004 | 85,700,748 |
| 10,000 | Hooks | 280,087 | 114,305,468 | 106,901,748 |
| 10,000 | Raw | 280,087 | 97,796,108 | 89,371,284 |

All 81 state-workload rows rejected the initial handles after unmount. The component test repeats removal/remount 100 times and verifies stale generations. Both immediate and pending async workloads end every cycle with zero tracked live effect guards and zero tracked live resource futures for all three implementations. Immediate loads completed; pending loads did not complete. A subsequent `--quick` run also asserted 100 live effect guards and, for pending loads, 100 live futures before unmount, so the cleanup assertions cannot pass because no work started. Full GPUI entity/subscription counts and attributed leak-free heap proof are **not** provided by these counters. Retained process allocations remain substantial even for plain GPUI and cannot be called component leaks or dismissed without attribution.

## Investigation of regressions

The first full run showed avoidable per-render work. Updating the existing props box in place and avoiding a mutable entity update/full hook scan during paint reduced allocations for 100 unchanged hook components from 3,047 to 2,847 per draw. The 50-hook case also benefits from avoiding the paint scan. The table above is the rerun after these changes.

Residual costs remain: keyed element bookkeeping, dynamic props/slot dispatch, runtime type checks, cloned state snapshots, registry validation on writes, and additional owned slot/props allocations at mount. These are source-level explanations and measured allocation differences, not sampled CPU attribution. At 10,000 instances, hook mount p50 is 14% higher, unchanged-parent p50 12% higher, and repeated replacement p50 20% higher than plain GPUI. At 100 instances with 50 hooks, p50 is 16% higher and p95 34% higher. The 1-instance leaf ratio is 25% but is particularly sensitive to small absolute times. Async frame p95 also exceeds the threshold in this run despite comparable medians.

The approximately 10% gate is therefore **not passed**. Further profiling and repeated controlled runs are required; the prototype must not be described as zero-overhead. Raw keyed state becomes substantially more expensive with many hooks because it creates one GPUI state entity/observer per slot; that does not waive the plain-GPUI comparison.

## Verification

Passed locally:

- 17 external-consumer component tests, including six compile-fail macro fixtures, real pointer-click dispatch, sibling state, stable reorder, props/defaults, another-app rejection, async setters, stale generations, parent identity, tabs, refs/memo/effects, context, managed resources, and explicit-layout memo caching.
- 5 focused internal checks for hook-order diagnostics and inspector data/bounds.
- 31 existing view-state tests; 20 CDP DOM tests, including hook metadata exposure; 82 doctests; the macro crate unit test.
- Focused Clippy with `-D warnings`. Rake formatting/lint pass; lint reports 14 pre-existing warnings and none in changed files.
- Native renamed-facade consumer check; explicit wasm32-unknown-unknown checks for the facade and renamed consumer.
- `gpui-fast` native library check; feature-disabled facade/macros library check.

Commands use `--offline --locked` locally after dependency fetch:

```sh
cargo test -p rake-app --features experimental-components,view-test-support,devtools --test function_components --test view_state
cargo test -p rake-app --features experimental-components,view-test-support,devtools --lib component::
cargo test -p rake-app-cli --bin rake cdp::dom
cargo test -p rake-app --features experimental-components,view-test-support,devtools --doc
cargo check -p rake-app -p renamed-facade --features rake-app/experimental-components,rake-app/view-test-support,renamed-facade/experimental-components --lib
cargo check -p rake-app -p renamed-facade --features rake-app/experimental-components,renamed-facade/experimental-components --target wasm32-unknown-unknown --lib
cargo check -p rake-app --features experimental-components,view-test-support,gpui-fast --lib
cargo check -p rake-app -p rake-macros --no-default-features --lib
cargo clippy -p rake-app --features experimental-components,view-test-support,devtools --lib --test function_components --bench function_components --example components -- -D warnings
cargo bench -p rake-app --features experimental-components,view-test-support --bench function_components
cargo bench -p rake-app --features experimental-components,view-test-support --bench function_components -- --native plain
cargo bench -p rake-app --features experimental-components,view-test-support --bench function_components -- --native hooks
cargo bench -p rake-app --features experimental-components,view-test-support --bench function_components -- --native raw
cargo bench -p rake-app --features experimental-components,view-test-support --bench function_components -- --native hooks mount 10000 1
```

Native arguments are `variant scenario instances hooks`. The matrix varies the
scenario over `mount`, `parent`, `leaf`, `all`, `reorder`, and `cycle` at each of
1, 100, 1,000, and 10,000 instances; `hooks` uses 100 instances with 1, 10, or 50
hooks. Repeat for `plain`, `hooks`, and `raw`.

## Simplification follow-up

The refactor removes 187 Rust lines across the component runtime, macro, tests, and benchmark harness. State and ref handles share one implementation; resources share loader access; the registry uses the already-resolved `slotmap` crate; hook access validates and downcasts once. Test hosts use typed props and shared update helpers. The public component syntax and all 17 consumer tests remain intact.

After the refactor, the 17 component tests, 5 internal checks, 31 existing view-state tests, focused Clippy, renamed-consumer checks, native `gpui-fast`, feature-disabled builds, and WASM compilation passed. All 51 quick headless benchmark cases completed, including stale-handle and effect/resource cleanup assertions. Formatting passed; Rake lint reported only the same 14 existing warnings. The native smoke attempt timed out; the earlier benchmark binary also timed out when rerun, so this follow-up does not establish native-window or performance acceptance. The full measurement matrices were not repeated.

## Remaining completion gates

- Automatic leaf render isolation and content-sized memoization are unresolved with the pinned renderer.
- The approximately 10% performance gate and heap/entity/subscription attribution remain unresolved.
- The macOS state matrix completed. Native async/effect measurements and the complete Linux matrix remain outstanding; no Linux host was available in this session.
- WASM compilation passed; this is not browser/GPU execution proof.
- Instruction counts were not collected: `xctrace list templates` fails because the active developer directory is CommandLineTools rather than a configured Xcode installation. No Linux `perf` is available.
- `blaze` is not installed/on PATH in this checkout environment. Hot reload was not run. The repository historical failed experiment is not evidence of the current implementation passing or failing Blaze.
- Actual virtual-list recycling remains to be tested in an application fixture.
