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

Heading sizes are 30, 24, 20, 16, 16, and 16 logical pixels. Classes and style
closures can override them. Containers have no browser margins or user-agent
stylesheet. `span` and `p` do **not** implement browser inline text flow; mixed
inline formatting needs a GPUI text component embedded as an expression.

## Attributes, styling and events

Attribute values are quoted strings or `{Rust expressions}`. The supported
attributes are `id`, `class`, `style`, `aria-label`, `aria-description`, `tabindex`,
the events below, and `src`/`alt` on images. `alt` maps to the image's accessible
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
children under an explicitly identified row can use generated IDs. The macro
cannot verify uniqueness or stability of dynamic IDs. Direct text/expression
children inside loops need an element wrapper with an explicit stable ID.
Embedded non-string expressions own
their own identity, including elements they return from nested macro invocations.
Reusable helper views rendered as siblings must also accept distinct root IDs.

## Current limits

Unknown tags, attributes, duplicate attributes, invalid utilities and malformed
markup are compile errors. This version does not implement input controls,
`disabled` buttons, links/forms, CSS selectors/cascade, HTML whitespace/entity
parsing, fragments, component-tag syntax, automatic signal tracking, or hot reload
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
cargo test -p rake --features experimental-view --doc --locked
cargo check -p rake-view -p renamed-facade --all-targets --locked
cargo check -p rake-view -p renamed-facade --lib --target wasm32-unknown-unknown --locked
```

The example increments/resets a counter, toggles a conditional history, displays
a keyed loop of the five most recent values, and embeds an ordinary GPUI element.
Native and Web use the same render method. Compilation and construction tests do
not establish GPU rendering, focus navigation or browser execution; those need a
running example on the selected host.

The example was smoke-tested on macOS: click/Space increment, Tab then Enter
reset, history hide/show, the five-row history limit, and accessible text/control
labels. Web compilation is checked; browser execution has not been verified.
