# Experimental function components

The stable markup reference is [View syntax](view.md). Function components are
available with `rake-app`'s opt-in `experimental-components` feature. This is a
source-checkout prototype for [issue #27](https://github.com/1bye/rake/issues/27);
the published alpha does not yet include it. Enable the feature on your local
Rake dependency. It is not enabled by default. This feature selects the pinned
`gpui-fast` backend for native and WASM builds. Use `rake::gpui` for GPUI types
so your components and application share the same backend.

```rust
use rake::{Scope, gpui::{IntoElement, SharedString}};

#[rake::component]
fn Counter(cx: &mut Scope, label: SharedString, start: i32) -> impl IntoElement {
    let (count, set_count) = cx.use_state(|| start);
    let doubled = cx.use_memo(count, |value| value * 2);

    rake::view! {
        <button
            id="increment"
            aria-label={format!("{label}: {count} ({doubled})")}
            on:click={move |_, _, cx| {
                let _ = set_count.update(cx, |value| *value += 1);
            }}
        >
            {format!("{label}: {count} ({doubled})")}
        </button>
    }
}

fn example() -> impl IntoElement {
    rake::view! { <Counter id="clicks" label="Clicks" start={0}/> }
}
```

## Ownership and React comparisons

| React | Rake |
| --- | --- |
| Function runs again during rendering | A generated GPUI entity calls the Rust function during rendering |
| `useState(initial)` | `cx.use_state(|| initial)` returns an owned snapshot and a `Copy` setter |
| Functional state update | `setter.update(cx, |value| *value += 1)` edits the latest stored value |
| `useRef` | `cx.use_ref(...)` retains data without notifying on mutation |
| Dependency array | One comparable Rust value; use a tuple for several dependencies |
| Effect cleanup | Return an owned closure from `use_effect` |
| Keyed child | Explicit `id`, scoped by the parent element path and component type |

Use hooks for local UI state such as disclosure state, selection, or a transient
request. Keep application/domain state in ordinary `Entity` fields when other
parts of the application own it, observe it, or must retain it after the component
disappears. One generated entity owns each mounted function component.

`count` is a snapshot for one render. A closure that captures it keeps that
snapshot. `set_count.update` changes the current stored value, so two updates do
not overwrite each other using an old captured count. Cloning a state snapshot
costs whatever that type's `Clone` costs; share large owned data when appropriate.

## Hook contracts

- `use_state` initializes once per mount and requires `Clone` for snapshots.
  Changing a `start` prop does not reset an existing slot.
- `use_ref` accepts non-`Clone` values. Its `read(cx, callback)` and
  `update(cx, callback)` access the current value, including during rendering.
  Ref updates do not notify the component.
- `use_memo(dependencies, compute)` requires comparable dependencies and a
  cloneable result. The callback receives `&dependencies`.
- `use_effect(dependencies, effect)` runs after paint and after the render borrow
  has ended. The callback receives `&mut gpui::App` and returns `()` or a cleanup
  closure. Cleanup runs before replacement and on unmount. Repeated renders
  before paint retain the latest pending effect. Unpainted components do not
  run effects.
- `use_context::<G>()` clones an installed GPUI global and observes its changes.
  `use_theme::<T>()` observes the installed `ThemeGlobal<T>` and returns `Rc<T>`.
- `use_resource(&runtime, dependencies, factory)` uses Rake's existing managed
  loading binding. The factory receives owned dependencies and `LoadContext`.
  Changed dependencies cancel and replace the previous loader. The returned
  `Resource` provides `read`, `retry`, and `cancel`, without `Clone` bounds on
  successful values or errors.

```rust
let request = cx.use_resource(&runtime, document_id, |id, load| async move {
    load.background_executor()
        .spawn(async move { std::fs::read_to_string(format!("{id}.txt")) })
        .await
});

let label = request.read(cx, |state| match state {
    rake::loading::LoadStatus::Ready(value) => value.clone(),
    rake::loading::LoadStatus::Failed(error) => error.to_string(),
    _ => "Loading…".into(),
})?;
```

Setters, refs, and resource handles are `Copy` and stay on the UI thread. They
accept `App`, `Context`, `AsyncApp`, and `Scope`. They do not keep their owner
alive. Stale handles, including handles used with another application, return
`ComponentError::Disposed`. Reused registry entries get a new generation, so an
old async callback cannot update a replacement component.

For additional async work, retain the task in an effect cleanup to cancel it on
unmount:

```rust
cx.use_effect((), move |cx| {
    let task = cx.spawn(async move |cx| {
        let value = fetch().await;
        let _ = set_data.set(cx, value);
    });
    move || drop(task)
});
```

Call hooks in the same order every render. Conditional hooks, early returns that
skip hooks, and changing the number of looped hook calls are invalid. The runtime
diagnoses count, kind, and storage-type changes. Swapping hooks with the same
kind and type cannot be distinguished; it is still incorrect.

## Props, identity, and rendering

The macro generates a props struct named after the function. `id` is reserved
for mount identity. Other parameters become cloneable props, converted with
`Into` when the function runs. String literals therefore work for `SharedString`
without changing existing `view!` component construction. Struct-update defaults
continue to work. Component functions currently require synchronous, non-generic
signatures with `&mut Scope` first and named prop parameters.

State survives parent renders and list reordering under stable keys. Changing a
key, component type, or keyed ancestor creates a fresh instance. Removing an
element releases its entity when GPUI retires that frame's element state.
Conditional branches, switched-out tabs, and virtualized rows that leave the
rendered tree reset on remount. GPUI cached subtrees retain their element state.
Keeping a route/page entity alive alone does not preserve removed function
components; put persistent tab data in that entity.

A setter updates its component's hook storage and notifies that component.
The retained renderer reuses unchanged ancestors and siblings automatically,
including content-sized components. Layout changes still propagate: growing a
child, changing inherited styles, or resizing the window can require rebuilding
surrounding layout.

`#[rake::component(memo)]` compares incoming props using `PartialEq` and reuses
an unchanged component when its parent renders again. It needs no explicit
width or height. State and observed context changes still redraw it. Without
`memo`, a parent render runs the component again with the latest props, as in
React. Use memoization for expensive children whose props usually stay equal.

The optional `.into_element().cached(style)` API remains available for explicit,
definite-size GPUI caching. Automatic retention does not guess or freeze a
component's dimensions. Tests cover independent siblings, intrinsic-size memo
components, layout reflow, after-paint effects, and disposal of virtualized rows.
A uniform list's measurement row remains mounted even when outside the viewport;
other removed rows release their hooks and reset on remount.

## Inspection and verification

With `devtools`, authored component boundaries expose up to 64 hook slots with
position, kind, and storage type. Scalar and string state/ref values have bounded
previews; other values remain opaque. Strings and type names are limited to 256
UTF-8 bytes, and metadata to 16 KiB per boundary. User state needs neither `Debug`
nor serialization. CDP exposes this information as `rakeHooks`.

```sh
cargo test -p rake-app --features experimental-components,view-test-support --test function_components
cargo test -p rake-app --features experimental-components,view-test-support,devtools --lib component::
cargo bench -p rake-app --features experimental-components,view-test-support --bench function_components
cargo bench -p rake-app --features experimental-components,view-test-support --bench function_components -- --headless hooks parent 1000 1
cargo bench -p rake-app --features experimental-components,view-test-support --bench function_components -- --native hooks
```

The native benchmark also accepts `plain`, `memo`, and `raw`, followed by optional
`scenario instances hooks` arguments, for example `--native hooks mount 10000 1`.
Scenarios are `mount`, `parent`, `leaf`, `all`, `reorder`, `cycle`, and `hooks`.
`--headless` accepts the same arguments and runs one case with 31 measured
samples. Use separate processes and alternate the variant order for comparisons;
the quick suite alone is not a performance acceptance result. All four variants
use the same renderer, recorded in each result. `hooks` preserves ordinary prop
rendering; `memo` measures equal-prop reuse comparable to retained plain entities.
Headless measurements cover
CPU render/layout/paint, allocations, retained process memory, and lifetime
checks; they do not measure GPU presentation. Async scenarios report frame times
and separate whole mount/settle/unmount cycle times.

Try the interactive counters, including reorder and remount behavior:

```sh
cargo run -p rake-app --features experimental-components,gpui_fast_platform/runtime_shaders --example components
```

The [benchmark report](../.agents/reports/react-function-components.md) records
measurements and remaining merge gates. Stable hook positions avoid
source-line keys, but do not establish Blaze compatibility or preserve arbitrary
Rust values across process restarts. Real Blaze edits require separate proof.

The manual [function-component workflow](../.github/workflows/function-components.yml)
enables the feature on macOS and Linux, runs consumer tests and lifetime checks,
and checks native, WASM, renamed-dependency, and feature-disabled builds. Its
Linux window smoke test uses Xvfb and Mesa Vulkan drivers; it does not
establish physical-display or hardware-GPU performance.

## Formatting

Run `rake format` after editing markup. See [Formatting Rust and view markup](format.md)
for exact layout rules, preservation guarantees, check mode, and rust-analyzer setup.
