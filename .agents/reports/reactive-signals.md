# Issue #27: signals prototype

Date: 2026-11-12. Branch: `feat/react-styled-function-component`. Upstream base:
`c18fe8d40979be0eb8986a95103e162e852750aa`. Results describe this working-tree
implementation, not a published release.

This replaces the hooks experiment following the maintainer's
[design review](https://github.com/1bye/rake/pull/48#issuecomment-6727565816) and the
[rewritten issue](https://github.com/1bye/rake/issues/27). The earlier
[hooks report](react-function-components.md) is historical. Its Linux, native,
and Blaze observations do not validate this implementation.

## Implementation and spike findings

The opt-in feature is now `experimental-reactive`. Stateful functions run setup
once, retain a generic `ComponentView`, and rerun only the final expression.
Stateless functions remain `RenderOnce`. Props use generated type-state builders,
including defaults, generics, named props, spreads, tracked inputs, events, and
rerunnable children. Existing overlay components and consumer fixtures use the
same builders. There is no parallel hooks runtime.

Signals use application-local typed registries, generational IDs, and GPUI
`Entity<T>` storage. Borrowed reads support non-Clone values. Component ownership
releases registrations, observers, tasks, providers, and cleanup callbacks at
unmount. Generation and application identity prevent stale writes from reaching
replacement state. App-owned registrations have an explicit `dispose` method.
The inspector exposes bounded named signal metadata without requiring `Debug`.

The spike found three important differences from the proposed pseudocode:

1. GPUI's public `AppContext` has no associated `Result`. Writes use a small
   `SignalContext` adapter and return `Option<R>` for `update`; stale writes are
   no-ops. `AsyncApp` can itself panic once its app is gone. A fallible app-gone
   result requires an upstream API change. `from_entity(cx, entity)` also needs
   the application to register ownership, so a context-free `From<Entity<T>>`
   implementation is not provided.
2. GPUI read tracking alone did not invalidate a cached component that reads a
   separate signal entity. Rake therefore tracks reads across render, layout,
   prepaint, and paint, and maintains subscriptions to notify the component.
   Dynamic dependencies replace old subscriptions; unchanged subscriptions are
   reused. The observer stack is an App global with scoped guards, not TLS.
   This avoids a GPUI fork but adds measurable work. Exposing dependency tracking
   upstream remains an alternative, not something this report benchmarks.
3. The plain cached benchmark also needed an explicit entity read in its host to
   register with this backend's invalidator. Without it, the cached row displayed
   stale data and produced misleadingly low timings. A new assertion failed on
   that baseline and passed after the fix. The final benchmark asserts the first
   row's rendered value after every leaf and bulk update in every variant.

The existing raw keyed-state collision fixture passes: siblings separated by
explicit IDs do not share state. Stable keys survive reorder and insertion;
removed branches, switched-out tab subtrees, and virtualized rows start fresh on
remount. A retained page entity alone does not retain removed descendants.

One entity per signal preserves direct GPUI interoperability and borrowed reads.
The 1/10/50-read cases quantify its cost, but an alternative in-view storage
implementation has not been benchmarked. This is an explicit remaining spike
comparison rather than evidence that entity-per-signal is the cheapest design.

## Verification

The affected macOS suites passed 259 tests: 178 library, 14 component consumers
(including compile-fail fixtures), 29 view syntax consumers, 7 component markup
consumers, and 31 view-state tests. One pre-existing manual library benchmark is
ignored. Coverage includes cached sibling isolation, current props, dynamic memo
dependencies and a 50-link chain, after-frame effects and cleanup, stale handles,
context, owned tasks, tracked-prop refetch, non-Clone resource values, failure and
retry, 100 successive input changes, cancelled late completions, and unmount.

Additional checks include:

- All targets for `rake-app`, the renamed facade, and the existing view example
  with the feature enabled; the view example also compiles without it.
- Feature-disabled libraries and default-backend libraries.
- Renamed dependency and WASM library consumers.
- gpui-fast with function components and native controls compiles on macOS.
- Macro/protocol/parser tests and separate default/reactive expansion snapshots.
- CDP named-signal property test.
- All 85 documentation examples; focused Clippy and rustdoc with `-D warnings`;
  Rake formatting.

The final Rake lint invocation reported zero diagnostics and zero denied findings.
Keyboard, focus, disabled-button, tab, popover, and dialog behavior is covered by
existing view-state tests. No new screen-reader, contrast, zoom, physical-window,
or Linux runtime verification is claimed.

## Measurement method

Apple M2 Pro MacBook Pro (Mac14,9), 16 GB RAM, macOS 27.3 (26C5035r), ARM64.
Rust 1.98.0 (`88d9e12ae`), optimized Cargo bench profile, pinned gpui-pre 0.3.11.
No GPUI dependency revision changed. The lockfile change adds the existing local
`rake-macros` dependency to `rake-ui` for `Props` derives.

The full suite has 174 cases. Ordinary cases discard five warmups and measure 31
samples; async mount/settle/unmount cases discard three warmups. This is one
sequential run on a shared development machine, without confidence intervals or
instruction counts. Headless measurements include CPU mutation, render, layout,
and paint, not GPU submission or presentation. Initial compilation is excluded.

| Variant | Ownership and rendering |
| --- | --- |
| Plain | Retained GPUI view with state in fields, no explicit cache |
| PlainCached | Same view with a definite cached box and a tracked entity read |
| Signals | Function component and per-value entities, caching disabled |
| Cached | Same signals with a definite cached box |
| Raw | Explicit keyed window state without Rake's component/observer machinery |

Rows have the same dimensions and content. All variants use the same backend.
The read-count cases rerender a parent with 100 rows. Fan-out uses one shared
source; the plain/raw memo-chain baseline computes equivalent arithmetic inline,
while signals maintain the dependency chain. Children cases render equivalent
nested content. Async baselines use ordinary GPUI tasks and `LoadState`, not
Rake's heavier application loading services.

Resource bursts perform 1, 10, or 100 writes in one application update. The last
request is completed, its rendered value asserted, and cancelled senders rejected.
Retry starts while a request is pending. Unmount occurs with another pending
request, whose completion must fail. Resource timings cover mutation and the
loading frame; completion settling is checked outside that timed interval.
`mount` and `cycle` both replace the entire row set; `reorder` reverses it, removes
one row, and inserts a fresh key. They are not cold-process startup measurements.

## Results

All 174 cases passed the value, cancellation, and lifetime assertions. Cells in
the main table are **p50 / p95 milliseconds**. No timing threshold was treated as
a correctness assertion.

| Rows | Scenario | Plain | PlainCached | Signals | Cached | Raw |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1,000 | mount | 8.91 / 11.38 | 11.03 / 12.58 | 10.92 / 14.32 | 18.75 / 24.98 | 8.30 / 8.56 |
| 1,000 | parent | 8.02 / 9.41 | 4.77 / 5.17 | 10.98 / 12.38 | 5.12 / 6.11 | 7.76 / 7.93 |
| 1,000 | leaf | 7.53 / 7.80 | 4.59 / 5.30 | 11.44 / 13.11 | 4.95 / 6.63 | 7.75 / 7.88 |
| 1,000 | all | 7.67 / 8.17 | 11.05 / 11.49 | 11.84 / 13.32 | 14.83 / 16.63 | 7.97 / 8.08 |
| 1,000 | reorder | 7.54 / 8.02 | 10.59 / 11.75 | 10.05 / 11.95 | 14.97 / 32.62 | 7.77 / 8.59 |
| 1,000 | cycle | 7.74 / 8.80 | 10.74 / 12.37 | 12.45 / 14.57 | 19.06 / 20.23 | 8.30 / 9.60 |
| 10,000 | mount | 77.67 / 92.92 | 348.85 / 379.49 | 153.61 / 168.81 | 673.20 / 875.65 | 95.35 / 104.57 |
| 10,000 | parent | 82.53 / 89.81 | 25.43 / 31.04 | 114.82 / 123.49 | 51.03 / 54.46 | 89.35 / 112.62 |
| 10,000 | leaf | 77.68 / 91.01 | 31.01 / 33.72 | 114.91 / 126.39 | 50.31 / 65.35 | 80.15 / 94.72 |
| 10,000 | all | 79.31 / 89.60 | 353.22 / 409.54 | 115.98 / 124.75 | 534.01 / 703.72 | 82.30 / 95.21 |
| 10,000 | reorder | 81.93 / 90.32 | 345.08 / 392.13 | 119.38 / 126.77 | 518.24 / 635.11 | 80.10 / 87.68 |
| 10,000 | cycle | 82.32 / 92.78 | 346.58 / 383.06 | 184.42 / 309.24 | 633.15 / 660.95 | 94.57 / 103.47 |

Mount memory includes the live window and backend caches. The following values
are **live bytes per initial row / allocations per replacement row**, without an
agreed acceptance budget:

| Rows | Plain | PlainCached | Signals | Cached | Raw |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 7,172 / 53.0 | 8,880 / 44.0 | 12,304 / 79.0 | 13,260 / 69.0 | 8,940 / 60.0 |
| 100 | 7,681 / 30.5 | 12,242 / 21.5 | 10,919 / 56.0 | 15,622 / 46.0 | 8,643 / 37.7 |
| 1,000 | 9,211 / 30.1 | 13,038 / 21.1 | 14,705 / 55.5 | 18,654 / 45.5 | 10,194 / 37.2 |
| 10,000 | 8,741 / 30.0 | 10,038 / 21.0 | 14,028 / 55.4 | 15,234 / 45.4 | 9,780 / 37.1 |

Additional uncached cases below use **p50 / p95 microseconds**:

| Scenario | Plain | Signals | Raw |
| --- | ---: | ---: | ---: |
| 100 rows, 1 reads each | 630.46 / 778.67 | 838.25 / 1112.58 | 623.33 / 653.75 |
| 100 rows, 10 reads each | 602.17 / 687.33 | 962.54 / 1309.71 | 909.75 / 925.33 |
| 100 rows, 50 reads each | 596.79 / 618.88 | 1487.88 / 1774.75 | 2165.83 / 2189.46 |
| Fan-out to 1 views | 11.12 / 18.67 | 14.17 / 14.33 | 12.08 / 14.71 |
| Fan-out to 100 views | 655.79 / 693.96 | 843.62 / 868.92 | 725.79 / 738.67 |
| Fan-out to 1000 views | 8109.58 / 8303.54 | 10414.00 / 13186.79 | 8851.25 / 9065.58 |
| Memo depth 1 | 10.96 / 11.29 | 14.67 / 14.92 | 11.92 / 12.29 |
| Memo depth 10 | 10.96 / 14.58 | 17.88 / 18.08 | 11.75 / 11.92 |
| Memo depth 50 | 11.04 / 11.21 | 33.12 / 33.83 | 11.79 / 11.88 |
| Rerunnable children | 18.67 / 19.04 | 21.67 / 22.04 | 19.54 / 19.71 |
| Resource burst, 1 writes | 8.54 / 8.67 | 11.50 / 11.58 | 9.62 / 9.88 |
| Resource burst, 10 writes | 9.21 / 9.33 | 12.04 / 15.88 | 10.46 / 11.00 |
| Resource burst, 100 writes | 14.29 / 14.96 | 16.50 / 16.58 | 15.96 / 20.83 |
| Retry in flight | 8.96 / 9.17 | 11.83 / 11.96 | 9.92 / 10.04 |

For 100 async components, these are **p50 / p95 milliseconds per whole
mount/settle/unmount cycle**; the measured frame p50 is shown in parentheses:

| Scenario | Plain | Signals | Raw |
| --- | ---: | ---: | ---: |
| effect_resource_async | 90.41 / 93.95 (0.29) | 106.34 / 127.66 (0.32) | 91.35 / 92.19 (0.30) |
| effect_resource_cancel | 31.47 / 32.54 (0.29) | 36.61 / 48.17 (0.33) | 31.63 / 32.10 (0.30) |

Representative allocator results for 1,000 rows and an unchanged-parent update:

| Variant | Allocations/update | Live bytes after mount | Live bytes after unmount |
| --- | ---: | ---: | ---: |
| Plain | 26,066 | 9,210,932 | 9,034,796 |
| PlainCached | 6,066 | 13,038,356 | 12,166,276 |
| Signals | 32,066 | 14,704,508 | 12,097,828 |
| Cached | 10,066 | 18,653,836 | 15,311,212 |
| Raw | 28,066 | 10,194,140 | 9,346,612 |

### Performance interpretation

The approximately 10% target is not met across the matrix. At 10,000 rows:

- Parent: Signals p50 is 39% above Plain; Cached is 101% above PlainCached.
- Leaf: Signals p50 is 48% above Plain; Cached is 62% above PlainCached.
- All: Signals p50 is 46% above Plain; Cached is 51% above PlainCached.

The extra keyed shell still executes during parent rendering even when its
cached child skips rendering. Props comparison and keyed-state lookup remain.
Uncached signal rows allocate six more times per row/update in this fixture;
cached unchanged-parent rows allocate four more times per row. Per-signal entity
storage and explicit subscription tracking also grow the 50-read working set.
These measured allocations and the required extra boundaries explain additional
work; they do not assign an exact CPU percentage to any one mechanism.

Bulk updates and reorder at 10,000 rows are expensive for both cached variants.
The corrected PlainCached baseline also reproduces that cost, so it is not solely
a signals regression. Caching benefits unchanged content and isolated updates;
it is not an unconditional win for a changing tree. The explicit opt-out remains
important. Rerunnable children and resource bursts add much smaller absolute
costs, while a 50-observer memo chain costs more than direct arithmetic.

Possible next experiments are a GPUI dependency-tracking API, in-view local
storage, and avoiding repeated keyed/props work on unchanged ancestors. None is
claimed as a measured optimization here. The current evidence supports keeping
the feature experimental, not waiving the performance gate.


## Ownership and acceptance limits

Every ordinary and dependency case asserts that Rake-owned component, signal,
observer, subscription, and pending-task counts return to its baseline. Shared
source signals are explicitly disposed after the dependency cases. Async cases
also assert zero live effect guards and resource futures; resource cases reject
late completions after unmount. These counts exclude GPUI caches and entities
that callers explicitly retain.

The counting allocator measures process-wide live bytes while a test window is
still alive. GPUI frame buffers, layout caches, and registry capacity remain after
component disposal. Retained bytes therefore are not an attributed leak count.
Zero Rake ownership counters and dropped value/future guards establish the tested
lifetimes, not a full heap leak audit.

Automatic cache inference covers explicit root width and height, including
percentages. Dynamic root styles, forwarded rest attributes, positioned or hidden
roots, class variants, inferred flex stretching, and content-sized roots are excluded. Rest forwarding currently
covers checked utility styling, accessible label/description/expanded state,
disabled state, and click handlers; other event kinds need explicit event props.
Paragraph text uses explicit signal reads. Stateful named props are passed as a
`props` value. The setup-read diagnostic is syntactic and cannot follow aliases.
These limits are documented rather than silently presented as the full proposed API.

The macro embeds setup tokens in mount identity and excludes the render tail.
That alone is not live reload proof. Blaze had not been installed for the original
benchmark run. The 2026-11-13 live trial below supplies fresh evidence for render
text and setup initializer edits; it does not establish arbitrary closure-layout
compatibility. The older failed Blaze experiment is not reused as current proof.
Real-window macOS/Linux benchmarks are follow-ups in the revised issue; Linux
execution and instruction counts were not collected in this run.

Keep this opt-in and review it as the requested spike/prototype. Performance
acceptance and the alternative storage experiment remain open; the live trial
below covers the two requested reload behaviors. No merge to the upstream default branch is implied by pushing this review
branch.

## Review resolution proposal (2026-11-13)

This revision completes the automatic-cache layout fix. The regression reproduces
an absolutely positioned 160 × 40 root moving its sibling from x=0 to x=160 on a
later parent render. The inference now declines positioned roots, hidden roots,
and class variants, preserving ordinary GPUI layout. The same test passes across
three parent renders. Parser coverage also retains inference for fixed dimensions,
percentages, and `size-*`. This fallback makes no render-isolation claim for the
excluded roots.

### Verification of this revision

263 feature tests passed: 181 library, 15 function-component consumers, 29 view,
7 component-markup, and 31 view-state tests; one existing manual benchmark remains
ignored. A further 29 macro/parser tests passed. The layout regression failed
before the fix and passed afterward. Rake formatting/lint, focused macro/parser
Clippy with warnings denied, feature-disabled compilation with `view-test-support`,
and the `wasm32-unknown-unknown` reactive library check passed.

The initial sandboxed library run could not bind the existing HTTP fixture's
localhost socket; the complete rerun with socket access passed. A plain native
check requires Apple's missing `metal` compiler on this machine, so the
feature-disabled check used the existing runtime-shader test feature. Neither
limitation prompted source or CI changes.

### Live reload verification

On 2026-11-13, the current CLI installed and verified the published Apple Silicon
bundle `1bye/blaze` `0.20261030.1-rake.1`, SHA-256
`aec30a92722bed32f7dd5d101520ab5554149339a66ac670a352065865457e10`.
No engine override or compiler patch was used. The standalone fixture depends on
this checkout's `rake-app`, with `experimental-reactive` and `view-test-support`;
its `live` and `devtools` features forward to Rake. Cargo checked the same fixture
successfully before the successful Blaze run.

The fixture uses `ControlConfig::FromEnvironment` and mounts this component under
`id="counter"` in a normal GPUI root:

```rust
#[rake::component]
fn Counter(cx: &mut Setup) -> impl IntoElement {
    let count = cx.signal(0i32);
    let owner = format!("Owner: {:?}", cx.entity_id());
    rake::view! {
        <section>
            <p>{format!("Version A count {}", count.get(cx))}</p>
            <p>{owner.clone()}</p>
            <button id="increment" aria-label="Increment counter"
                on:click={move |cx| count.update(cx, |value| *value += 1)}>
                Increment
            </button>
        </section>
    }
}
```

Run it with `rake dev --live --manifest-path <fixture>/Cargo.toml
--bin rake-signals-live-review --json --no-open --require-state`. Use the existing
`scripts/native_test_client.py` control client to click `increment` three times,
read `inspect_tree`, then apply the two edits sequentially. Check `diagnostics`
for `live_patch_applied` and compare the runtime PID, visible count, and owner ID.

| Stage | Edit | PID | Visible count | Component owner |
| --- | --- | --- | --- | --- |
| Before edits | Three real preview-input clicks | 94624 | 3 | `EntityId(4v1)` |
| Render only | `Version A` → `Version B` in the final expression | 94624 | 3 | `EntityId(4v1)` |
| Setup | `cx.signal(0i32)` → `cx.signal(10i32)` | 94624 | 10 | `EntityId(7v1)` |

Both edits emitted `live_patch_applied`: patch sequences `1794589409728` and
`1794589410068`, respectively. The executable UUID remained
`uuid:821dc413d4453cee80dec3d4599975ef`; both events reported zero restored state
entries. The visible text after setup was `Version B count 10`. This establishes
state retention for the render edit and component remount for the setup edit,
without a process restart or checkpoint restoration.

The initial fixture attempt omitted the `AppContext` trait import and was refused
by Blaze; Cargo identified the missing import. The corrected fixture imports it
and uses `AnyView::from(cx.new(...))` at the app root. A temporary harness keyword
collision was also fixed before either patch was applied. Those failed setup
attempts are not counted as compatibility failures or successful reloads.

Evidence stays in temporary files: `/private/tmp/rake-signals-live-review/`,
`/private/tmp/rake-live-check.py`, `/private/tmp/rake-live-check.log`, and
`/private/tmp/rake-review-live-final.{jsonl,stderr}`. No raw JSONL is committed.
This is a real-window functional trial, not a performance measurement, Linux
proof, or a guarantee that closure-layout changes can be patched in place.

### API scope requiring agreement

The following are proposed follow-ups, not agreed reductions to #27. This revision
does not complete these four API extensions. Existing explicit reads remain
reactive when performed in the render expression.

| Proposed follow-up | Current supported form | Acceptance criteria |
| --- | --- | --- |
| Reactive paragraph content | `<p>{count.get(cx).to_string()}</p>` | Accept `Signal<T: Display>` and `Memo<T>` directly inside paragraphs, headings, and styled inline runs; preserve run boundaries, accessible text, and invalidation without requiring unrelated callers to acquire an app context. |
| Reactive built-in attributes | Direct `Value<bool>` for `disabled`; explicit render-time reads for other attributes | Define the supported attribute matrix and accept plain values, signals, and memos consistently; test updates to accessible names/states as well as visual attributes, including cached views. |
| Complete rest forwarding | Utility styles, accessible label/description/expanded, disabled, and click | Forward every supported undeclared `aria-*` and `on:*` attribute with typed handlers; specify precedence, diagnose unsupported attributes, and retain disabled keyboard/pointer suppression. |
| Flex-stretch cache inference | Explicit independent width and height; normal rendering otherwise | Compare cached and uncached bounds under row/column parents, stretched and non-stretched alignment, constrained dimensions, and sibling insertion; infer only when the parent constraints establish both dimensions. |

These follow-ups need maintainer agreement before treating this PR as the agreed
MVP. Content-sized caching remains the separate backend follow-up already listed
in #27. No GitHub follow-up issues have been created by this report.

### Performance next step

No performance exception is requested or assumed. Keep the approximately 10%
update/re-render gate open. The next experiment is a benchmark-only alternative
that stores local signals in the owning component while retaining the current
public handles. Compare it with entity-per-signal storage on the same checkout,
backend, machine, and optimized profile; do not replace production storage before
there is evidence that the alternative improves it.

Run the existing 1/10/50-read cases and mount, unchanged-parent, single-leaf, bulk
update, reorder, and repeated-unmount cases at 1, 100, 1,000, and 10,000 instances.
Use plain GPUI, plain cached GPUI, and raw keyed state as controls. Assert visible
updates and state identity in every variant. Report p50/p95, allocations, live and
retained bytes, and component/signal/subscription/task counts; ownership counts
must return to baseline. Repeat the measurements independently three times rather
than accepting one noisy run. Report mount bytes and allocations per instance for
an explicit budget decision, separately from update ratios.

If local storage meets the gate while preserving borrowed reads, interoperation,
disposal, and async behavior, implement it and rerun the full 174-case suite.
Otherwise isolate dependency/subscription and keyed-shell overhead next, and
bring measured tradeoffs back for review. Explanations of overhead do not close
acceptance. Real-window macOS/Linux measurements remain separate follow-ups from
the revised issue. This report does not declare the PR ready to merge.

## Reproduce

```sh
cargo bench -p rake-app --locked --features experimental-reactive,view-test-support --bench function_components
cargo bench -p rake-app --locked --features experimental-reactive,view-test-support --bench function_components -- --headless cached leaf 1000 1
cargo test -p rake-app --locked --features experimental-reactive,view-test-support,devtools --lib --test function_components --test view --test view_components --test view_state
```

Raw output remains in temporary storage. No JSONL report artifacts are committed.
