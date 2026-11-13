# Experimental function components

Enable `experimental-reactive` on a local `rake-app` dependency to use
`#[rake::component]` and `rake::reactive`. This source-checkout prototype follows
[issue #27](https://github.com/1bye/rake/issues/27)'s signals redesign. It replaces
the earlier `experimental-components` hooks API and is not enabled by default.
Use `rake::gpui` so the application and its components share the selected backend.
The stable markup reference remains [View syntax](view.md).

```rust
use rake::{gpui::{IntoElement, SharedString}, reactive::Setup};

#[rake::component]
fn Counter(label: SharedString, start: i32, cx: &mut Setup) -> impl IntoElement {
    let count = cx.signal(start);
    let doubled = cx.memo(move |cx| count.get(cx) * 2);

    rake::view! {
        <button
            id="increment"
            class="w-40 h-10"
            aria-label={format!("{label}: {}", count.get(cx))}
            on:click={move |cx| count.update(cx, |value| *value += 1)}
        >
            {label} ": " {count} " (" {doubled} ")"
        </button>
    }
}
```

Use it as `<Counter id="clicks" label="Clicks" start={0}/>` inside `view!`.
The complete runnable example is `crates/rake/examples/components.rs`.

## Setup, rendering, and ownership

Statements before the final expression execute once per mount. The final
expression executes on each render, with prop names rebound to their current
values and the real GPUI `window` and `cx`. Put computations that should rerun
inside that expression, or use `cx.memo` for a reusable derived value.

A component with a final `&mut Setup` parameter owns one typed GPUI `Render`
entity. `Setup` dereferences to its real `Context`, so ordinary GPUI operations
remain available. Without `Setup`, a component is a stateless `RenderOnce`
struct. `#[rake::component(view)]` forces a view boundary without local state.
The render closure and props remain concrete generic types. Children and event
callbacks use shared closures because those values must be cloneable props.

Use signals for state whose lifetime belongs to a mounted UI component. Keep
application data, retained tab state, and large collections in ordinary entity
fields when another part of the application owns them. Signals add a registry,
an entity per value, and dependency subscriptions; they are not free fields.
There is no thread-local runtime or separate batching API.

| React | Rake |
| --- | --- |
| Component body runs on every render | Setup runs once; the final expression renders again |
| `useState` returns a render snapshot | `Signal` is a handle; read its current value through `cx` |
| Functional setter | `count.update(cx, \|value\| *value += 1)` |
| `useMemo` dependency array | `cx.memo` tracks signals read by its callback |
| `useEffect` and returned cleanup | `cx.effect` and `cx.on_cleanup` inside the callback |
| Hook call order must remain fixed | Setup can create signals conditionally or in loops |
| A keyed child retains state | Explicit stable `id`, component type, and parent path retain state |

## Signals and derived values

`Signal<T>` and `Memo<T>` are `Copy` handles, including when `T` is not `Clone`.
Copying a handle does not retain its component. `read(cx)` borrows `&T`,
`get(cx)` clones a `T: Clone`, and `with(cx, callback)` provides a scoped borrow.
`Signal::entity(cx)` returns the underlying GPUI entity for interoperability.
Retaining that entity explicitly can extend the value's lifetime beyond unmount.

`set` and `update` notify readers. `set_if_changed` requires `PartialEq` and
suppresses equal writes. Writes work with `App`, `Context`, and `AsyncApp` through
`SignalContext`; disposed or foreign-application handles do nothing, and
`update` returns `None`. Reads after disposal panic, like using an invalid owner;
use `is_alive` when retaining handles outside their component.

`cx.signal(value)` is owned by the component. `Signal::new(cx, value)` and
`Signal::from_entity(cx, entity)` are application-owned registrations: call
`dispose(cx)` when that explicit ownership should end. Generational IDs prevent
an old handle from reaching a new value in a reused slot.

`cx.memo(move |cx| ...)` tracks synchronous signal reads, replaces dependencies
when branches change, and notifies only when its `PartialEq` result changes.
A direct read of a recognized local signal in setup is rejected because it
would capture only the mount-time value. This syntax check does not follow
aliases or calls through arbitrary helper functions.

`{count}` and `{doubled}` render displayable values in element children.
Paragraphs use Rake's text-run builder, so format a read there, for example
`<p>{count.get(cx).to_string()}</p>`. `disabled={busy}` accepts either a bool or a
`Signal<bool>`. Signal reads in other attributes can be explicit, such as
`aria-expanded={expanded.get(cx)}`.

## Props, events, and children

With the feature enabled, component tags use type-state builders. Handwritten
component structs add `#[derive(rake::Props)]`. Required fields must be supplied;
`Option<T>` and `bool` default to `None` and `false`. `#[prop(default)]` uses
`Default`, and `#[prop(default = expression)]` evaluates only when omitted.
Setters accept `Into` by default; `#[prop(no_into)]` opts out. `{..base}` applies
explicit attributes over the base, preserving source evaluation order.
Generic components retain their bounds. Stateful props must be `Clone + 'static`.

```rust
#[rake::component]
fn Greeting(
    name: SharedString,
    #[prop(default = 1)] repetitions: usize,
) -> impl IntoElement {
    rake::view! { <div>{name.repeat(repetitions)}</div> }
}
```

A stateless `fn Card(props: CardProps)` delegates to the named struct's derived
builder, allowing `<Card title="..."/>`. A stateful named-props component accepts
that struct explicitly: `<Card id="card" props={props}/>`.

A setup prop normally keeps its mount-time value. Mark it `#[prop(track)]` to
receive a `Memo<T>` in setup that follows the parent; its type requires
`PartialEq`. In the render expression it is still the current ordinary prop.
Untracked props never reset state merely because their input changes.

`#[event] click: ClickEvent` creates an optional `Event<ClickEvent>` prop.
Invoke it with `click.call(event, window, cx)`. `on:click` accepts
`move |cx| ...`, `move |event, cx| ...`, or the full GPUI handler.
`#[rest] rest: Attrs` forwards root utility styling, accessible label/description,
expanded state, disabled state, and clicks. Other event kinds must be declared
as explicit event props. Forwarded styles cannot contain interactive variants.
Disabled buttons retain the built-in keyboard and interaction suppression.

Stateful `Children` and markup slots are rerunnable factories receiving the real
window and app. Capture owned values, and clone non-`Copy` values inside a factory
when each render consumes them. Stateless children can remain one-shot elements.
Keep render parameters named `window` and `cx` when nested markup uses these
factories or reactive attributes.

## Effects, async work, and resources

`cx.effect(move |window, cx| ...)` runs after a frame and again when a synchronous
tracked read changes. Register cleanup with `cx.on_cleanup` inside the effect;
it runs before replacement and on unmount. `cx.on_cleanup` in setup runs once at
unmount. Cleanup has a live `App`, and component signals remain available while
cleanup executes.

`cx.spawn(async move |cx| ...)` owns its foreground task until completion or
unmount. Explicit GPUI task handles created through other APIs keep their normal
GPUI ownership rules. Move blocking work to a background executor.

```rust
let weather = cx.resource(move |cx| {
    let city = city.get(cx);
    async move { fetch_weather(city).await }
});
```

A resource starts at mount. Reads before the factory returns its future are
tracked; reads inside the future are not. Input changes and `retry(cx)` cancel
the previous task. `read(cx)` borrows the existing `LoadStatus`; `latest(cx)`
retains the last successful value during loading or failure without requiring
`Clone`. Generation checks reject stale results. Unmount cancels outstanding
work, including a request that has never completed.

## Context, identity, and caching

`cx.provide(value)` and `cx.context::<T>()` perform a mount-time lookup through
function-component parents. Provide a signal when descendants should react to
changes. A handwritten GPUI view interrupts that parent chain. App-wide state
uses `provide_global(cx, value)` and the `ReactiveApp::global_signal` extension.

Stable keys preserve state through reorder. Removing a branch, switching out a
tab subtree, or virtualizing a row away releases its component when GPUI retires
unused element state. A later remount starts fresh. Retaining only the page
entity does not retain removed descendants. Cached subtrees preserve their
mounted state; a uniform list's measurement row may remain mounted offscreen.

Automatic caching is conservative: a literal root class with explicit width
and height, including percentages, supplies GPUI's cached layout box. Use
`#[rake::component(cached = false)]` to disable it or
`#[rake::component(cached = "w-40 h-10")]` to provide a definite box. Dynamic styles,
rest attributes, positioned roots (`absolute`, `relative`, or offsets), `hidden`,
class variants, and inferred flex stretching do not automatically qualify.
These roots render normally so caching cannot change their layout or visibility.
An explicit cache style must also omit positioning, `hidden`, and variants;
supplying dimensions alone does not make a positioned root safe to cache.
Content-sized caching is a separate GPUI follow-up. Equal comparable props reuse
the cached view; props without a known `PartialEq` implementation conservatively
invalidate it. Signal dependencies still invalidate the owning view.

Without caching, GPUI may redraw ancestors and siblings. The pinned backend also
requires Rake subscriptions for cached function-component reads, collected across
render, layout, and paint. State locality alone does not guarantee render isolation.

## Backend constraints and inspection

The pinned GPUI `AppContext` has no associated `Result` type. This prototype uses
`SignalContext` and `Option` for disposed writes rather than the proposed
`C::Result`. `AsyncApp` itself can panic after its application disappears; Rake
cannot turn that into a fallible write through this backend's public API.
Registration also requires `cx`, so `from_entity(cx, entity)` replaces the
proposed context-free `From<Entity<T>>` conversion.

The macro hashes setup tokens into mount identity. Setup edits therefore change
the key; render-only edits do not. A native trial with the pinned Blaze bundle
verified that a render text edit retained count 3 and the component entity, while
a setup initializer edit remounted it with count 10, in the same process. See the
[live verification report](../.agents/reports/reactive-signals.md#live-reload-verification).
This covers edits that preserve closure layout; structural changes remain subject
to Blaze's restart policy.

With `devtools`, the inspector and CDP expose `rakeSignals` with binding names,
kinds, types, and bounded scalar/string previews. Other values remain opaque
without requiring `Debug`. Metadata is limited to 64 bindings and 16 KiB per
boundary. Diagnostic ownership counts under `view-test-support` cover Rake-owned
components, registered signals, observers, dependency subscriptions, and tasks;
they exclude GPUI caches and externally retained entities.

## Verification

```sh
cargo test -p rake-app --locked --features experimental-reactive,view-test-support,devtools --lib --test function_components --test view_state
cargo bench -p rake-app --locked --features experimental-reactive,view-test-support --bench function_components
cargo bench -p rake-app --locked --features experimental-reactive,view-test-support --bench function_components -- --headless cached leaf 1000 1
cargo bench -p rake-app --locked --features experimental-reactive,view-test-support --bench function_components -- --native signals all 100 1
cargo run -p rake-app --features experimental-reactive,gpui_platform/runtime_shaders --example components
```

Variants are `plain`, `plain-cached`, `signals`, `cached`, and `raw`. Headless
measurements include CPU mutation/render/layout/paint and allocator counts, not
GPU presentation. The full suite also measures fan-out, memo chains, children,
async disposal, rapid resource inputs, and retry. Native measurements remain a
follow-up in the revised issue. See the [signals report](../.agents/reports/reactive-signals.md)
for measured costs, verification, and remaining acceptance gaps.
