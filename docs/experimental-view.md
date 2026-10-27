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

Each invocation returns one concrete GPUI element or component and requires exactly one root
tag. Closing tags must match; empty elements can use `<div/>`. Text must be a Rust
string literal (`"Hello"`) or a Rust expression (`{format!("Hello, {name}")}`).
Literal text is not interpolated automatically. Rust comments work within markup.
Outside paragraphs, expressions may contain statements and must produce an `IntoElement` value. They
can embed existing GPUI builders, `Entity` handles, or `RenderOnce` components;
clone retained handles explicitly when needed. There is no string-based HTML
parser, Dioxus runtime, virtual DOM, or JavaScript execution.

Outside paragraphs, literal strings and expressions whose GPUI element is `SharedString` or a static
string become identified, accessible `gpui::Text` nodes. Other embedded elements
keep their own identity and semantics. Literal-only buttons/headings get an
accessible label automatically; supply `aria-label` for dynamic or icon buttons.

| Tags | GPUI behavior |
| --- | --- |
| `div`, `section`, standalone `span` | Flex-column containers |
| `p` | One composed GPUI text layout inside a styled paragraph container |
| `span`, `strong`, `em`, `br` inside `p` | Inline style ranges and explicit line breaks, without separate layout boxes |
| `h1`–`h6` | Containers with a heading accessibility role/level, bold text, and default sizes |
| `ul`, `ol`, `li` | Containers with list/list-item accessibility roles; no generated bullets or numbering |
| `button` | Button role, pointer cursor, and tab stop; GPUI dispatches mouse clicks and focused Enter/Space activation |
| `img` | GPUI image with required `src` and `alt`; accepts no children |
| Uppercase names, such as `Badge` or `ui::Card` | Typed Rust component structs implementing `IntoElement`, normally through `RenderOnce` |
| `view` | Mounts a borrowed, caller-owned `Entity<V>` through `entity={&self.editor}`; accepts no other attributes or children |

Heading sizes are 30, 24, 20, 16, 16, and 16 logical pixels. Classes and style
closures can override them. Containers have no browser margins or user-agent
stylesheet. Paragraphs compose inline text as described below. A standalone
`span` remains a layout container; heading and button children still use the
ordinary element-child rules.

## Attributes, styling and events

Attribute values are quoted strings or `{Rust expressions}`. The supported built-in
attributes are `id`, `class`, `style`, `aria-label`, `aria-description`, `tabindex`,
`focus`, `scroll`, `key-context`, `tab-navigation`, typed `on:Action` handlers,
`disabled` on buttons, and `src`/`alt` on images. Containers and buttons also accept
typed `role`, `aria-keyshortcuts`, and `aria-expanded`. `alt` maps to the image's accessible
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
IDs and an image's source are configured first. A button's `disabled` predicate is
evaluated once before its builder. Focus/tab-index configuration and the optional
Tab handler apply after authored styles and handlers, irrespective of attribute
order. Other attribute expressions run once, before children, in written order;
macro-authored button event handlers are only constructed when enabled.

| Attribute | Callback event |
| --- | --- |
| `onclick` | `gpui::ClickEvent` |
| `onkeydown`, `onkeyup` | `gpui::KeyDownEvent`, `gpui::KeyUpEvent` |
| `onmousemove` | `gpui::MouseMoveEvent` |
| `onmousedown`, `onmouseup` | `gpui::MouseDownEvent`, `gpui::MouseUpEvent`, left button |

Callbacks receive `(event, window, app)` as in GPUI; use `cx.listener(...)` for
access to view state. Events retain GPUI propagation and default-prevention
behavior. Keyboard events require the appropriate focus path.

## Paragraph composition, wrapping and truncation

`p` collects text into one UTF-8 buffer with styled runs, rendered by GPUI's
text shaping system. End truncation measures the actual styled runs and font
fallbacks, and cuts only at grapheme boundaries. Its only inline tags are `span`, `strong` (bold), `em` (italic),
and `br` (newline). They do not create layout boxes or independent element IDs.
Nested formatting inherits and combines. Spaces remain explicit in string
literals; markup indentation adds no spaces, and HTML entities are not decoded.

```rust
use rake::gpui::{self, rgb, rgb_to_hsla};
let name = "Ada";
let _paragraph = rake::view! {
    <p wrap={true} style={|s| s.w(gpui::px(320.))}>
        "Hello, "<strong>{name}</strong>". "
        <span style={gpui::HighlightStyle::color(rgb_to_hsla(rgb(0x365849)))}>
            "One "<em>"continuous"</em>" paragraph."
        </span>
        <br/>"An explicit second line."
    </p>
};
```

Paragraph expressions must implement `AsRef<str>`, including `String`, `&str`,
`SharedString`, and borrowed string values. Use `format!` for numbers or other
`Display` values. Arbitrary `IntoElement` values, components, widgets, images,
and block tags are not paragraph content. `if`/`if let` and `for` work inside
paragraphs, evaluating only selected text once in written order. Inline loops
need no IDs because their output is text, not independently retained elements.
An empty `<p/>` is supported.

Inline `class` accepts only `font-normal`, `font-medium`, `font-semibold`,
`font-bold`, `italic`, `not-italic`, and `underline`. Inline `style` takes a typed
`gpui::HighlightStyle` value, **not** the builder closure used on layout tags.
It supports GPUI run properties such as color, weight, italics, underline,
strikethrough, and background. `font-family` accepts a GPUI font-family string.
Attributes apply after the tag's default style in written order. Font size,
line height, alignment, dimensions, padding and other layout belong on `p`.
Inline IDs, events and accessibility overrides are rejected; `br` accepts no
attributes or children.

| Paragraph attribute | Behavior |
| --- | --- |
| `wrap={true}` | Soft-wrap at the available width; this is the default |
| `wrap={false}` | Disable soft wrapping; explicit newlines remain |
| `ellipsis={true}` | Enable GPUI end-ellipsis and clip overflow |
| `max-lines={n}` | Apply GPUI's line clamp and clip overflow; zero normalizes to one |

```rust
# use rake::gpui;
# let title = "A long document title";
# let preview = "A longer preview that can wrap onto several lines.";
let _text = rake::view! {
    <section>
        <p wrap={false} ellipsis={true} style={|s| s.w(gpui::px(240.))}>
            {title}
        </p>
        <p wrap={true} max-lines={3} ellipsis={true}
            style={|s| s.w(gpui::px(320.))}>
            <strong>"Preview: "</strong>{preview}
        </p>
    </section>
};
```

A constrained width is necessary for wrapping and truncation. For multiline
ellipsis, specify `max-lines`; ellipsis without a line limit truncates to the
available width. A line cap without ellipsis clips without adding an ellipsis.
Ellipsis without `max-lines` targets one visible line, including when the source
contains explicit breaks. `ellipsis={false}` does not add truncation styling. These controls use the normal
attribute/style order, so later raw GPUI styling can override earlier settings.
This phase supports end truncation; start/middle truncation and text transforms
through raw style builders are not implemented. The paragraph keeps one accessible
text node containing the full composed text,
even when the visual preview is truncated. Selection and interactive links are
not implemented by this phase.

**Migration:** `p` previously accepted arbitrary layout children and embedded
GPUI elements. Move those into a `div` or `section`, or embed an existing custom
text component there. Use inline `span` styles only for text refinements inside
`p`; standalone spans keep their existing layout API. Paragraph IDs still scope
under their parent and follow the normal loop identity rules when a loop emits
multiple paragraphs.

## Keyboard, focus and accessibility

```rust
# use rake::gpui::{self, FocusHandle, IntoElement};
gpui::actions!(editor, [Save]);

fn toolbar(focus: &FocusHandle, saving: bool) -> impl IntoElement {
    rake::view! {
        <section focus={&focus} key-context="Editor" tab-navigation={true}
            role={gpui::accesskit::Role::Group} aria-label="Editor commands"
            on:Save={|_: &Save, _, _| { /* call the application's save operation */ }}>
            <button disabled={saving} aria-label="Save document"
                aria-keyshortcuts="Control+S"
                style={|s| s.focus_visible(|s| s.border_2())}
                onclick={|_, window, cx| window.dispatch_action(Box::new(Save), cx)}>
                "Save"
            </button>
        </section>
    }
}
```

`on:Save` and qualified paths such as `on:crate::actions::Save` expand to typed
GPUI `on_action::<Save>` listeners. Action types must implement `gpui::Action`;
handlers use the normal GPUI signature and accept `cx.listener(Self::save)`.
Register `cx.bind_keys([gpui::KeyBinding::new("secondary-s", Save, Some("Editor"))])`
once during application setup. `key-context` supplies GPUI's keymap context on
the focused dispatch path; it does not register shortcuts. `secondary` means
Command on macOS targets and Control on other targets. `aria-keyshortcuts` is
accessibility metadata only; supply the actual platform shortcut yourself.

`tab-navigation={true}` installs a bubbling handler for unmodified Tab and
Shift+Tab. It calls GPUI's window-wide `focus_next`/`focus_prev`, then prevents the
default and stops propagation. A descendant or earlier authored handler can
consume the event with `cx.stop_propagation()` or `window.prevent_default()`.
Nested navigation handlers therefore advance only once. This is opt-in and does
not establish a focus trap or constrain navigation to the container's subtree.
Set initial window focus once so the handler is on the keyboard dispatch path.

Buttons get a retained focus handle and `tabindex={0}` by default, including when
`focus` supplies a stored handle. Use `tabindex={-1}` to skip sequential navigation
while allowing explicit focus; nonnegative indices use GPUI's ordering. On other
elements, `tabindex` establishes a tab stop and can configure an explicitly supplied
handle. `focus={&self.focus}` tracks a stored `FocusHandle`;
`scroll={&self.scroll}` tracks a stored `ScrollHandle`. Construct handles once in
the owning view, not in `render`. Configure overflow styling for scrolling.
Both attributes require shared borrows of stored variables, fields or indexed
entries; temporary constructors and method-call results are compile errors.
Focus attributes take precedence over raw focus configuration in `style`.

`disabled={condition}` is supported on `button` only. Disabled buttons skip Tab,
do not take mouse focus, suppress macro-authored event/action handlers, block
mouse and unmodified Enter/Space activation, and expose AccessKit's disabled flag
without activation/focus actions. A button disabled while focused keeps its focus
anchor so Tab and enclosing actions continue to work. It is not automatically
blurred. Application action handlers must still check whether their operation is
available: an enclosing handler can be invoked by a shortcut or other application
code. Disabled appearance remains explicit, for example an opacity style.

Use `aria-label` for dynamic/icon controls, `aria-description` for extra context,
`aria-expanded={open}` on disclosure buttons, and typed
`role={gpui::accesskit::Role::Group}` for containers with custom semantics.
Literal-only buttons/headings get labels automatically. GPUI's
`focus_visible(|s| ...)` supplies keyboard-visible styling. Button contents should
be noninteractive; embed existing GPUI widgets for more complex controls.
Arbitrary Rust builder code inside `style` is an escape hatch and remains the
caller's responsibility, including any event listeners it installs directly.

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

Untaken branches and empty-loop bodies do not evaluate their children. Built-in
layout tags add each child directly to their GPUI parent without an intermediate
tree or child vector. Paragraphs build their one text buffer and styled-run list. Paired component tags collect their authored children in the
component's `Vec<AnyElement>`. Large lists should use an embedded GPUI virtual
list instead of eagerly generating every item.

Each built-in tag gets a GPUI ID, either explicit or derived from its source site and
position in the macro. GPUI scopes IDs under their nearest identified ancestor.
Outside paragraph text, elements emitted directly by a loop **must** have explicit IDs, including elements
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
and parents constructed outside the macro are not enumerated. A `RenderOnce`
component is also opaque until GPUI renders it: its root ID cannot be checked by
the surrounding parent's construction-time registry. Its authored child vector
is checked before being handed to the component. Components must forward unique
IDs to their own roots; duplicate IDs across deferred component roots remain the
caller's responsibility. Release builds
omit the duplicate-ID map and checks. Stable key values remain the caller's
responsibility.

## Typed components, children and slots

Uppercase tags construct ordinary Rust structs: `<Badge .../>` or
`<ui::Badge .../>` becomes `Badge { ... }` or `ui::Badge { ... }`. Paths can begin
with `crate`, `self`, `super`, or `::`; the final name must start with an uppercase
letter. Components normally implement GPUI's `RenderOnce` and derive
`gpui::IntoElement`. No additional component macro or runtime is required.

```rust
use rake::gpui::{self, AnyElement, ElementId, prelude::*};

#[derive(IntoElement)]
struct Card {
    id: ElementId,
    header: AnyElement,
    footer: Option<AnyElement>,
    children: Vec<AnyElement>,
}

impl RenderOnce for Card {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        rake::view! {
            <section id={self.id} class="flex flex-col gap-4 p-4 rounded-md">
                {self.header}
                <div class="flex flex-col gap-2" style={|s| s.children(self.children)}/>
                if let Some(footer) = self.footer {
                    {footer}
                }
            </section>
        }
    }
}

let _card = rake::view! {
    <Card id={"profile".into()}
        header={rake::view! { <h2>"Profile"</h2> }.into_any_element()}
        footer={Some(rake::view! { <p>"Saved locally"</p> }.into_any_element())}>
        <p>"Your profile information."</p>
        <button onclick={|_, _, _| {}}>"Edit"</button>
    </Card>
};
```

Properties map directly to Rust fields. Values have their declared types, with
ordinary Rust moves, borrows, and visibility checks. Rust rejects missing, unknown,
or incorrectly typed properties. There is no automatic `.into()`, cloning,
optional-property default, builder-method lookup, or property spreading. A quoted
value is an `&'static str`; use `label={"Title".into()}` for `SharedString`.
GPUI's `RenderOnce` requires `'static` components: own values or explicitly clone
retained handles instead of storing references to render-time locals.
Generic component types can be inferred from their fields; explicit generic
arguments in tag paths are not supported.

Self-closing tags initialize only their listed properties. Paired tags always
supply `children: Vec<AnyElement>`, including an empty vector for `<Stack></Stack>`.
This is a convention for arbitrary nested GPUI elements, not a `ParentElement`
requirement on the component struct. `if`, `for`, string-to-accessible-text
conversion, and retained `<view>` mounts work within those children. Alternatively,
pass `children={existing_children}` on a self-closing tag. Combining that property
with a paired tag is rejected. Authored properties evaluate once in written order,
followed by the selected children in render order. Component rendering happens
later through GPUI's usual `RenderOnce` lifecycle.

Named slots are ordinary typed properties, such as the required `header` and
optional `footer` above. Supply `None` explicitly for an absent optional slot.
The component chooses where to render each slot and its children. Dedicated
`<slot>` syntax is not implemented. Callback properties work the same way:
`on_save={cx.listener(Self::save)}` must match the component field's Rust type.
Component properties must be Rust field names, using `r#` for keywords as needed.
Namespaced `on:Action` and hyphenated built-in attributes such as `aria-label` do
not apply to component tags. Even `class`, `style`, `disabled`, `focus`, and `id`
are ordinary fields on a component, and the component decides how to use them.

Forward a component's `id: ElementId` property to an identified GPUI root, as
`Card` does above. Component tags do not insert layout wrappers, generate IDs, or
create retained state. Direct component roots inside loops require an explicit
`id` property, using stable data keys. Keep repeated instances' root IDs unique,
including instances created outside loops. Changing or discarding those roots
has the same state/focus consequences as ordinary GPUI rendering.

Component structs are reconstructed when their parent renders. Keep persistent
data and stateful widgets in caller-owned entities and mount them with
`<view entity={&self.editor}/>` inside component content. The macro constructs the
component value; it never calls `cx.new` or retains it in a component cache.
User-authored property expressions and `RenderOnce` implementations remain normal
Rust code and can allocate if they explicitly choose to.

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

## Scrolling, virtual lists and overlays

Retained scroll handles work through `scroll`; GPUI virtual lists embed through
`{expression}`. The experimental `rake::overlay` module adds `OverlayHost`,
`OverlayAnchor`, `Popover` and `Dialog` as ordinary typed components, with retained
state, live anchor geometry, stacked dismissal and modal focus containment. See
[scrolling and overlays](experimental-overlays.md) for ownership, snippets and limits.
The example opens a retained 10,000-document virtual list from a nested dialog.

## Current limits

Unknown tags, attributes, duplicate attributes, invalid utilities and malformed
markup are compile errors. This version does not implement input controls,
links/forms, automatic shortcut labels, general-purpose focus traps outside the overlay controls, relationships such as
`aria-labelledby`, or a complete ARIA attribute vocabulary, selectable text or inline links,
CSS selectors/cascade, HTML whitespace/entity
parsing, fragments, dedicated slot syntax, automatic signal tracking, or hot reload
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
cargo test -p rake --features experimental-view --test view --test view_components --test view_paragraphs --locked
cargo test -p rake --features view-test-support --test view_state --locked
cargo test -p rake --features experimental-view --doc --locked
cargo check -p rake-view -p renamed-facade --all-targets --locked
cargo check -p rake-view -p renamed-facade --lib --target wasm32-unknown-unknown --locked
```

The example uses reusable `components::Card` and `components::Badge` components.
Card accepts typed header/footer slots and collects the counter controls and
retained history into its body. Badge derives its label and appearance from
ordinary typed properties. Paragraphs demonstrate nested bold/italic runs,
colored dynamic text, Unicode, a two-line preview, and a single-line ellipsis.
The example routes clicks and shortcuts through typed actions. Command+I on
macOS targets (Control+I elsewhere) increments, Escape resets, H toggles history,
R reverses rows, and L locks/unlocks editing. Reset and Reverse are disabled when
unavailable; locking disables Increment and Reset. Tab/Shift+Tab skip disabled
controls and expose visible focus styling. History rows are retained entities
with their own focus and click counts; B or Enter/Space activates a focused row.
The list uses a stored scroll handle. Resetting or hiding history explicitly
returns focus to the counter root before rows disappear. Native and Web use the
same render method.

The updated example was smoke-tested on macOS: Enter and Command+I activation,
locked mouse/keyboard activation, Tab skipping disabled controls, native disabled
accessibility state, and row activation after a typed reorder. The component
follow-up also verified header/body/footer layout, dynamic Badge properties, and
retained row activation/focus after reordering inside Card. Paragraph composition
was checked on macOS for nested bold/italic runs, dynamic colored text, Unicode,
single- and two-line ellipses, and complete untruncated accessibility values. Web compilation
is checked; browser input and screen-reader interaction have not been verified.

The opt-in `view-test-support` feature enables GPUI's own deterministic TestPlatform.
Its frame tests exercise keyed element state, implicit button focus, retained
entity data/selection, explicit focus and scroll offsets across parent rerenders,
reordering and insertion. Component frame tests check forwarded root IDs, implicit
focus and keyed state across reorder/insertion, and retained child ownership.
Paragraph frame tests cover shared inline flow, width changes, explicit breaks,
line limits, and GPUI truncation. Keyboard frame tests cover typed actions, Tab/Shift+Tab,
negative indices, consumed events, disabled mouse/keyboard activation, and
keyboard-visible styling. The platform's window/text/GPU backend is a test double;
these checks do not establish native editor or browser input behavior.
