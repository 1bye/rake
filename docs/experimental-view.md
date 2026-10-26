# Experimental HTML-like GPUI views

Enable `rake = { ..., features = ["experimental-view"] }` to use `rake::view!`.
The feature enables GPUI, including when default features are disabled. The macro
generates ordinary GPUI builders and Rust control flow. State stays in your GPUI
entities, handlers use GPUI events, and changes still need `cx.notify()`.
This API is experimental and may change.

```rust
use rake::gpui::{Context, IntoElement, Render, Window, rgb};

struct Counter { count: usize }

impl Render for Counter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <div class="flex flex-col gap-4 p-6"
                style={|s| s.bg(rgb(0x18232b)).text_color(rgb(0xf0f4e8))}>
                <h1>"Counter"</h1>
                <p>{format!("Count: {}", self.count)}</p>
                <button id="increment" class="px-4 py-2 rounded-md"
                    onclick={cx.listener(|this, _, _, cx| {
                        this.count += 1;
                        cx.notify();
                    })}>
                    "Increment"
                </button>
            </div>
        }
    }
}
```

## Elements and values

Each invocation returns one concrete GPUI element and requires exactly one root
tag. Closing tags must match; empty elements can use `<div/>`. Text must be a Rust
string literal (`"Hello"`) or a Rust expression (`{format!("Hello, {name}")}`).
Literal text is not interpolated automatically. Rust comments work within markup.
Expressions may contain statements and must produce an `IntoElement` value. They
can embed existing GPUI builders, `Entity` handles, or `RenderOnce` components;
clone retained handles explicitly when needed. There is no string-based HTML
parser, Dioxus runtime, virtual DOM, or JavaScript execution.

Literal strings and expressions whose GPUI element is `SharedString` or a static
string become identified, accessible `gpui::Text` nodes. Other embedded elements
keep their own identity and semantics. Literal-only buttons/headings get an
accessible label automatically; supply `aria-label` for dynamic or icon buttons.

| Tags | GPUI behavior |
| --- | --- |
| `div`, `section`, `span` | Flex-column containers |
| `p` | Flex-column container with a paragraph accessibility role |
| `h1`–`h6` | Containers with a heading accessibility role/level, bold text, and default sizes |
| `ul`, `ol`, `li` | Containers with list/list-item accessibility roles; no generated bullets or numbering |
| `button` | Button role, pointer cursor, and tab stop; GPUI dispatches mouse clicks and focused Enter/Space activation |
| `img` | GPUI image with required `src` and `alt`; accepts no children |
| `view` | Mounts a borrowed, caller-owned `Entity<V>` through `entity={&self.editor}`; accepts no other attributes or children |

Heading sizes are 30, 24, 20, 16, 16, and 16 logical pixels. Classes and style
closures can override them. Containers have no browser margins or user-agent
stylesheet. `span` and `p` do **not** implement browser inline text flow; mixed
inline formatting needs a GPUI text component embedded as an expression.

## Attributes, styling and events

Attribute values are quoted strings or `{Rust expressions}`. The supported
attributes are `id`, `class`, `style`, `aria-label`, `aria-description`, `tabindex`,
`focus`, `scroll`, the events below, and `src`/`alt` on images. `alt` maps to the image's accessible
label; `aria-label` can override it when supplied after `alt`.
An empty image label (`alt=""`) marks it as decorative and omits its accessibility role.

`class` must be a literal string. It expands into a checked subset of GPUI
utility methods, in the order written. These are utilities, not stylesheet class
names or a complete Tailwind implementation:

- Layout: `flex`, `block`, `grid`, `hidden`, `flex-col`, `flex-row`, `flex-wrap`,
  `flex-1`, `grid-cols-1` through `grid-cols-12`.
- Alignment: `items-start`, `items-center`, `items-end`, `justify-start`,
  `justify-center`, `justify-end`, `justify-between`.
- Size: `w-full`, `h-full`, `size-full`, `min-w-0`, `min-h-0`.
- Spacing: `p`, `px`, `py`, `pt`, `pb`, `pl`, `pr`, `m`, `mx`, `my`, `mt`, `mb`,
  `ml`, `mr`, and `gap`, followed by `-0` through `-8`, or `-12`, `-16`, `-20`,
  `-24`, `-32`. Values use GPUI's rem-based spacing scale.
- Decoration: `rounded`, `rounded-none`, `rounded-sm`, `rounded-md`, `rounded-lg`,
  `rounded-xl`, `rounded-full`; `border`, `border-0`, `border-1`, `border-2`;
  `shadow-sm`, `shadow-md`, `shadow-lg`; `cursor-pointer`.
- Text: `text-xs`, `text-sm`, `text-base`, `text-lg`, `text-xl`, `text-2xl`,
  `text-3xl`, `text-left`, `text-center`, `text-right`; `font-normal`,
  `font-medium`, `font-semibold`, `font-bold`.

`style={|s| ...}` receives the GPUI builder through `FluentBuilder::map` and can
use its typed styling API, including colors, dimensions, hover and focus styles.
It should return the same builder type and preserve the element's identity.
Dynamic styles belong here; CSS strings and external stylesheets are unsupported.
Classes and style closures apply in attribute order, so later settings win.
IDs and an image's source are configured first, irrespective of attribute order.
Other attribute expressions run once, before children, in written order.

| Attribute | Callback event |
| --- | --- |
| `onclick` | `gpui::ClickEvent` |
| `onkeydown`, `onkeyup` | `gpui::KeyDownEvent`, `gpui::KeyUpEvent` |
| `onmousemove` | `gpui::MouseMoveEvent` |
| `onmousedown`, `onmouseup` | `gpui::MouseDownEvent`, `gpui::MouseUpEvent`, left button |

Callbacks receive `(event, window, app)` as in GPUI; use `cx.listener(...)` for
access to view state. Events retain GPUI propagation and default-prevention
behavior. Keyboard events require the appropriate focus path. Buttons establish
their own tab stop; other elements can set `tabindex={0}` or configure a focus
handle in the style closure. The macro installs no application-wide tab keymap;
use GPUI's focus navigation setup for your application.

`focus={&self.focus}` calls GPUI's `track_focus` with a stored `FocusHandle`.
`scroll={&self.scroll}` calls `track_scroll` with a stored `ScrollHandle`. Construct
these handles once in the owning view, not in `render`. Configure tab stops on a
stored focus handle with `.tab_stop(true)` and configure scrolling with an overflow
style. Both attributes require shared borrows of stored variables, fields or
indexed entries; temporary constructors and method-call results are compile errors.

## Conditions, loops and identity

Write `if`/`else if`/`else` and `for` directly among children. Each branch or loop
body may contain zero or more siblings. `if let` and destructured loop patterns
work. Conditions and iterators are Rust expressions; wrap struct literals in
parentheses as with ordinary Rust control flow.

```rust
# use rake::gpui::IntoElement;
# fn rows(rows: &[(usize, String)], visible: bool) -> impl IntoElement {
rake::view! {
    <section>
        if visible {
            <ul class="gap-2">
                for (id, label) in rows {
                    <li id={("row", *id)}>{label.clone()}</li>
                }
            </ul>
        } else {
            <p>"Hidden"</p>
        }
    </section>
}
# }
```

Untaken branches and empty-loop bodies do not evaluate their children. Each child
is added directly to its GPUI parent; the macro allocates no intermediate node
tree or child vector at runtime. Large lists should use an embedded GPUI virtual
list instead of eagerly generating every item.

Each tag gets a GPUI ID, either explicit or derived from its source site and
position in the macro. GPUI scopes IDs under their nearest identified ancestor.
Elements emitted directly by a loop **must** have explicit IDs, including elements
in conditional branches immediately inside that loop. Prefer stable data keys,
such as `id={("row", row.id)}`, over positions when rows can reorder. Nested
children under an explicitly identified row can use generated IDs. Direct text/expression
children inside loops need an element wrapper with an explicit stable ID.
Embedded non-string expressions own
their own identity, including elements they return from nested macro invocations.
Reusable helper views rendered as siblings must also accept distinct root IDs.

Debug builds check the actual GPUI IDs of direct children added to each
macro-authored parent, including children from loops, taken conditions and
embedded GPUI roots. A duplicate panics before layout with the ID and both source
sites. IDs can repeat under different identified parents, and untaken branches
are not checked. Detection is local to direct child roots; opaque GPUI descendants
and parents constructed outside the macro are not enumerated. Release builds
omit the duplicate-ID map and checks. Stable key values remain the caller's
responsibility.

## Retained component ownership

Keep stateful GPUI entities in their owning view or keyed data model. Mount them
with a shared borrow:

```rust
# use rake::gpui::{Entity, IntoElement, Render};
# fn retained<V: Render>(editors: &[Entity<V>]) -> impl IntoElement {
rake::view! {
    <section>
        for editor in editors {
            <view entity={&editor}/>
        }
    </section>
}
# }
```

`view` clones the borrowed entity handle, preserving its `EntityId`. It never
calls an initializer or creates an entity. A direct `view` in a loop uses that
entity's own stable GPUI ID and needs no separate `id`. Reordering the same
entities under the same parent retains their identity. Mounting the same entity
twice under one parent is a duplicate-ID error in debug builds.

The `entity` value must borrow a stored place, such as `&self.editor`,
`&row.editor`, or `&self.editors[index]`. Expressions like `&cx.new(...)` and
`self.editor.clone()` are rejected with an ownership diagnostic. Put styles and
additional identity scopes on a surrounding element; `view` itself accepts only
`entity`. Existing `{expression}` embedding remains available, and the caller is
responsible for any allocations or entity construction in arbitrary Rust code.

Owned entity data, including editor contents and selection, stays with that
entity. Stored focus/scroll handles keep their identity; keyed mounted elements
reuse GPUI's existing element state. Changing an ID or an identified ancestor,
moving a component to another parent, or unmounting it can reset implicit element
state. Unmounting can blur focus; the macro does not focus hidden components or
restore focus automatically. An entity kept by its owner retains its data even
while absent from the rendered tree; dropping/replacing the owner's handle ends
that lifetime normally. There is no global component cache or additional state
runtime.

## Current limits

Unknown tags, attributes, duplicate attributes, invalid utilities and malformed
markup are compile errors. This version does not implement input controls,
`disabled` buttons, links/forms, CSS selectors/cascade, HTML whitespace/entity
parsing, fragments, named component-tag syntax, automatic signal tracking, or hot reload
of markup. Existing GPUI components remain available through `{expression}`.
Images follow the application's GPUI asset/HTTP configuration.

Automatic Rake inspection boundaries are not inserted. A returned styled element
can use the existing `.inspect(...)` extension; descendants need their own
boundaries to appear in Rake's Elements panel.

## Example and checks

```sh
cargo run -p rake-view
python3 scripts/build-web.py --example view
python3 scripts/serve-web.py --example view --port 8083

cargo test -p rake-macros --locked
cargo test -p rake --features experimental-view --test view --locked
cargo test -p rake --features view-test-support --test view_state --locked
cargo test -p rake --features experimental-view --doc --locked
cargo check -p rake-view -p renamed-facade --all-targets --locked
cargo check -p rake-view -p renamed-facade --lib --target wasm32-unknown-unknown --locked
```

The example increments/resets a counter, toggles a conditional history, and mounts
the five most recent entries as caller-owned entities. Each row has its own click
counter and focus handle. Click a row, then press R or use Reverse order: the row's
data follows its entity, and the R shortcut keeps keyboard focus on that row.
The list uses a stored scroll handle. The example also embeds an ordinary GPUI element.
Native and Web use the same render method. Compilation and construction tests do
not establish GPU rendering, focus navigation or browser execution; those need a
running example on the selected host.

The example was smoke-tested on macOS: click/Space increment, Tab then Enter
reset, history hide/show, the five-row history limit, and accessible text/control
labels. Web compilation is checked; browser execution has not been verified.
The retained-row follow-up was also checked on macOS: a row kept its local click
count after reversing order and adding a sibling, and Space still activated that
same row after an R-key reorder.

The opt-in `view-test-support` feature enables GPUI's own deterministic TestPlatform.
Its frame tests exercise keyed element state, implicit button focus, retained
entity data/selection, explicit focus and scroll offsets across parent rerenders,
reordering and insertion. The platform's window/text/GPU backend is a test double;
these checks do not establish native editor or browser input behavior.
