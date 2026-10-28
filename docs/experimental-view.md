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
                    on:click={cx.listener(|this, _, _, cx| {
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
tag. Closing tags must match; empty elements can use `<div/>`. Text can be unquoted
(`Hello world`), a Rust string literal (`"Hello"`), or a Rust expression
(`{format!("Hello, {name}")}`).
Literal text is not interpolated automatically. Rust comments work within markup.
Outside paragraphs and headings, expressions may contain statements and must produce
an `IntoElement` value. They can embed existing GPUI builders, `Entity` handles, or `RenderOnce` components;
clone retained handles explicitly when needed. There is no string-based HTML
parser, Dioxus runtime, virtual DOM, or JavaScript execution.

Outside paragraphs and headings, literal strings and expressions whose GPUI element
is `SharedString` or a static string become identified, accessible `gpui::Text` nodes. Other embedded elements
keep their own identity and semantics. Literal-only buttons get an
accessible label automatically; supply `aria-label` for dynamic or icon buttons.

| Tags | GPUI behavior |
| --- | --- |
| `div`, `section`, standalone `span` | Flex-column containers |
| `nav`, `header`, `footer`, `main`, `aside`, `article` | Flex-column containers with Navigation, Banner, ContentInfo, Main, Complementary and Article roles |
| `hr` | Full-width, one-point rule; no separator role in this AccessKit version |
| `svg` | SVG asset leaf with required `path`; decorative unless aria-label is nonempty |
| `p` | One composed GPUI text layout inside a styled paragraph container |
| `span`, `strong`, `em`, `br` inside `p` or headings | Inline style ranges and explicit line breaks, without separate layout boxes |
| `h1`–`h6` | Composed text with a heading role/level, bold defaults, and the full composed accessible name |
| `ul`, `ol`, `li` | Containers with list/list-item accessibility roles; no generated bullets or numbering |
| `button` | Button role, pointer cursor, and tab stop; GPUI dispatches mouse clicks and focused Enter/Space activation |
| `img` | GPUI image with required `src` and `alt`; accepts no children |
| Uppercase names, such as `Badge` or `ui::Card` | Typed Rust component structs implementing `IntoElement`, normally through `RenderOnce` |
| `view` | Mounts a borrowed, caller-owned `Entity<V>` through `entity={&self.editor}`; accepts no other attributes or children |

Heading sizes are 30, 24, 20, 16, 16, and 16 logical pixels. Classes and style
closures can override them. Containers have no browser margins or user-agent
stylesheet. Paragraphs compose inline text as described below. A standalone
`span` remains a layout container; buttons keep ordinary element children.
Headings use the same composed inline text as paragraphs.

SVGs accept `path`, `class`, conditional classes, `style`, `aria-label` and
`aria-description`. For example, `<svg path="icons/search.svg" class="size-4"
aria-label="Search" style={|s| s.text_color(rgb(0xffffff))}/>` uses GPUI's asset
source and current text color. SVGs have no macro ID and are exempt from loop-ID
requirements. Wrap them in an identified element for retained identity, focus or
events. SVGs and rules reject children; style a rule's color explicitly, for
example with `style={|s| s.bg(rgb(0x61717c))}`.

## Attributes, styling and events

Attribute values are quoted strings or `{Rust expressions}`. Bare `disabled`,
`wrap`, `ellipsis`, `tab-navigation` and `aria-expanded` mean `={true}`. Component
props can also be bare booleans. Other attributes require a value. A braced bare
identifier in attribute position is shorthand: `<div {id}/>` means `id={id}`.
It preserves the property's normal type and ownership rules; in particular,
`{focus}` does not automatically borrow a handle. The supported built-in
attributes are `id`, `class`, `style`, `aria-label`, `aria-description`, `tabindex`,
`focus`, `scroll`, `key-context`, `tab-navigation`, typed `on:Action` handlers,
`disabled` on buttons, `tooltip`, `src`/`alt` on images, and `path` on SVGs.
Containers and buttons also accept
typed `role`, `aria-keyshortcuts`, and `aria-expanded`. `alt` maps to the image's accessible
label; `aria-label` can override it when supplied after `alt`.
An empty image label (`alt=""`) marks it as decorative and omits its accessibility role.

`class` accepts a literal string or an `if`/`if let`/`match` expression whose
leaves are literal strings. It expands into a checked subset of GPUI
utility methods, in the order written. These are utilities, not stylesheet class
names or a complete Tailwind implementation:

Utility strings are resolved at compile time. Conditional classes evaluate once
between their preceding and following authored attributes.

<!-- view-utilities:start -->

- `flex`, `block`, `grid`, `hidden`, `flex-col`, `flex-row`.
- `flex-wrap`, `flex-1`, `items-start`, `items-center`, `items-end`, `justify-start`.
- `justify-center`, `justify-end`, `justify-between`, `w-full`, `h-full`, `size-full`.
- `min-w-0`, `min-h-0`, `overflow-hidden`, `overflow-scroll`, `overflow-x-scroll`, `overflow-y-scroll`.
- `rounded-none`, `rounded-sm`, `rounded-md`, `rounded-lg`, `rounded-xl`, `rounded-full`.
- `border-0`, `border-1`, `border-2`, `shadow-sm`, `shadow-md`, `shadow-lg`.
- `cursor-pointer`, `text-xs`, `text-sm`, `text-base`, `text-lg`, `text-xl`.
- `text-2xl`, `text-3xl`, `text-left`, `text-center`, `text-right`, `rounded`.
- `border`, `shrink-0`, `grow`, `grow-0`, `flex-none`, `relative`.
- `absolute`, `items-stretch`, `self-start`, `self-center`, `self-end`, `justify-around`.
- `truncate`, `whitespace-nowrap`, `overflow-x-hidden`, `overflow-y-hidden`, `border-t`, `border-t-0`.
- `border-t-1`, `border-t-2`, `border-b`, `border-b-0`, `border-b-1`, `border-b-2`.
- `border-l`, `border-l-0`, `border-l-1`, `border-l-2`, `border-r`, `border-r-0`.
- `border-r-1`, `border-r-2`, `font-normal`, `font-medium`, `font-semibold`, `font-bold`.
- `grid-cols-N`, `p-N`, `px-N`, `py-N`, `pt-N`, `pb-N`.
- `pl-N`, `pr-N`, `m-N`, `mx-N`, `my-N`, `mt-N`.
- `mb-N`, `ml-N`, `mr-N`, `gap-N`, `gap-x-N`, `gap-y-N`.
- `w-N`, `h-N`, `size-N`.
- `top-N`, `right-N`, `bottom-N`, `left-N`, `inset-N`, `leading-none`.
- `p-13`, `gap-1.5`, `w-[320px]`, `h-[0.75rem]`, `size-[50%]`.
- `w-1/2`, `h-2/3`, `basis-1/3`, `-mt-1`, `-mx-[4px]`, `-top-2`.
- `text-[11px]`, `text-[0.75rem]`, `text-[#abc]`, `bg-[#123456]`, `border-[#12345678]`.
- `leading-[20px]`, `opacity-35`, `opacity-[0.35]`, `font-[JetBrains_Mono]`.
- `rounded-[4px]`, `border-[2px]`, `border-t-[2px]`.
- `min-w-[12px]`, `max-w-[50%]`, `min-h-[1rem]`, `max-h-[240px]`.
- `group/card`, `hover:bg-[#fff]`, `active:opacity-50`, `focus:border-2`.
- `focus-visible:text-[#000]`, `in-focus:font-bold`, `disabled:opacity-35`.
- `group-hover/card:bg-[#fff]`, `group-active/card:opacity-50`.

<!-- view-utilities:end -->

`N` accepts any nonnegative whole or half step, with one step equal to 0.25rem;
`grid-cols-N` uses 1–12. Existing scale values keep their GPUI convenience methods.
Length values in brackets require `px`, `rem` or `%`; text sizes, border widths
and radii accept only px/rem. Width, height and basis accept fractions with
denominators 2, 3, 4, 5, 6 and 12. Only margins and insets accept negatives.
Colors accept `#rgb`, `#rrggbb` and `#rrggbbaa`, inside brackets. Opacity uses
0–100 in steps of 5, or `[0..1]`. Font-family underscores become spaces.
Utilities map to the pinned GPUI builder methods. `truncate` and `whitespace-nowrap`
are raw GPUI text styling; composed paragraphs/headings also provide explicit
`wrap`, `ellipsis` and `max-lines` controls.

Interaction variants use `hover:`, `active:`, `focus:`, `focus-visible:` and
`in-focus:`. Named groups use `group/card`, `group-hover/card:` and
`group-active/card:`. Each interaction kind is merged into one GPUI style call,
including contributions from conditional class attributes. Variants apply after
authored attributes. `disabled:` is button-only and applies base styles at its
written position. Variants cannot stack, and reject group and overflow-scroll
utilities because those configure interactivity. GPUI supports only one named
group per group-hover/group-active kind on an element. SVG interaction classes
require a surrounding container.

A token scan rejects a style closure calling `.hover(..)` when the element also
has `hover:` classes, and similarly for the other interaction kinds. This is a
heuristic: named functions and indirect calls are opaque; GPUI's debug assertion
remains the backstop. Put a kind's refinements entirely in classes or in style.

Literal branches can nest, select arbitrary utilities and merge variants:

```rust
let selected = true;
let _ = rake::view! {
    <div class={if selected { "p-4 hover:opacity-50" } else { "p-2 hover:opacity-100" }}/>
};
```

Conditions, scrutinees and guards evaluate once in authored attribute order;
untaken nested conditions are skipped. Missing `else` means no classes.
Patterns cannot bind variables: use `Some(_)`, `_`, or qualified paths such as
`Option::None` / `Status::Ready`. Qualifying unit variants and constants avoids
confusing their identifiers with bindings. Match exhaustiveness is checked by
rustc. Variables, function calls and `format!` cannot produce class strings.
Inline text accepts the same literal-branch selection with its restricted run
utilities; interaction variants remain unsupported inside runs.

Errors quote each bad class and its ordinal position in the literal (for example,
`2nd class ` followed by `justify-betwen`). Nearby utility names and variants
receive suggestions such as `justify-between` and `hover:`. Every bad class is
reported, including classes in untaken literal branches; stable Rust spans
underline the whole literal. Value errors explain the expected units or range.

Conditional classes use a literal utility name: `class:shrink-0={fixed}` or
`class:font-bold={selected}`. Each condition is evaluated once, and the utility
applies in attribute order only when true. Unknown/dynamic names are errors.
Inline text tags reject conditional classes; select a `HighlightStyle` through
`style={...}` there instead.

`style={|s| ...}` receives the GPUI builder through `FluentBuilder::map` and can
use its typed styling API, including colors, dimensions, hover and focus styles.
It should return the same builder type and preserve the element's identity.
Other dynamic styles belong here; CSS strings and external stylesheets are unsupported.
Classes and style closures apply in attribute order, so later settings win.
IDs and an image's source are configured first. A button's `disabled` predicate is
evaluated once before its builder. Focus/tab-index configuration and the optional
Tab handler apply after authored styles and handlers, irrespective of attribute
order. Other attribute expressions run once, before children, in written order;
macro-authored button event handlers are only constructed when enabled.

| Attribute | Callback event / value |
| --- | --- |
| `on:click` | `gpui::ClickEvent` |
| `on:key-down`, `on:key-up` | `gpui::KeyDownEvent`, `gpui::KeyUpEvent` |
| `on:mouse-move` | `gpui::MouseMoveEvent` |
| `on:mouse-down`, `on:mouse-up` | Left-button `MouseDownEvent` / `MouseUpEvent` |
| `on:right-mouse-down`, `on:right-mouse-up` | Right-button events |
| `on:middle-mouse-down`, `on:middle-mouse-up` | Middle-button events |
| `on:any-mouse-down` | `MouseDownEvent` for any button |
| `on:mouse-down-out` | Any-button mouse down outside the element, during GPUI capture |
| `on:mouse-up-out` | Left-button mouse up outside the element, during GPUI capture |
| `on:hover` | `&bool` indicating hover enter/leave |
| `on:scroll-wheel` | `gpui::ScrollWheelEvent` |
| `on:drop` | Typed payload callback; annotate its first parameter, e.g. `|value: &Document, _, _| ...` |
| `on:drag` | `(payload, constructor)`; constructor takes `(&T, Point<Pixels>, &mut Window, &mut App)` and returns `Entity<V>` |
| `tooltip` | GPUI builder `Fn(&mut Window, &mut App) -> AnyView` |

Lowercase `on:*` names select built-in events. A path ending in an uppercase
identifier remains a typed action (`on:Save`, `on:crate::actions::Save`). Unknown
events report the supported names. A drag tuple evaluates once when its handler
is constructed; payload ownership follows GPUI. Explicit callback parameter types
or a named constructor function help inference when building the tuple separately.

The legacy aliases `onclick`, `onkeydown`, `onkeyup`, `onmousemove`, `onmousedown`
and `onmouseup` remain accepted with deprecation warnings pointing at the
attribute. Migrate them to the corresponding canonical `on:*` names. Disabled
buttons skip every macro-authored event/action builder, including drag/drop and
hover. `tooltip` remains available when disabled, so controls can explain why.

Most event callbacks receive `(event, window, app)` as in GPUI; use `cx.listener(...)` for
access to view state. Events retain GPUI propagation and default-prevention
behavior. Keyboard events require the appropriate focus path.

## Formatting

Rustfmt formats surrounding Rust but does not format `view!` bodies. Once a tag
has more than about three attributes, put one attribute on each line. Extract
long styling closures and listeners into named bindings or methods. A named style
closure may need a GPUI builder type annotation when Rust cannot infer it.

For example, this inspector-style control is difficult to scan:

```rust
# use rake::gpui::{self, IntoElement};
# fn dense(selected: bool) -> impl IntoElement {
rake::view! {
    <button id="details" class="px-3 rounded-md" aria-label="Show details" style={|s| s.h(gpui::px(28.)).bg(gpui::rgb(if selected { 0x365849 } else { 0x22323d })).hover(|s| s.bg(gpui::rgb(0x526857)))} on:click={|_, _, _| {}}>"Details"</button>
}
# }
```

The same control can keep its layout, appearance and handler easy to find:

```rust
# use rake::gpui::{self, prelude::*};
# fn formatted(selected: bool) -> impl IntoElement {
let details_style = move |s: gpui::Stateful<gpui::Div>| {
    s.h(gpui::px(28.))
        .bg(gpui::rgb(if selected { 0x365849 } else { 0x22323d }))
        .hover(|s| s.bg(gpui::rgb(0x526857)))
};
let show_details = |_: &gpui::ClickEvent, _: &mut gpui::Window, _: &mut gpui::App| {};

rake::view! {
    <button
        id="details"
        class="px-3 rounded-md"
        aria-label="Show details"
        style={details_style}
        on:click={show_details}
    >
        "Details"
    </button>
}
# }
```

For stateful listeners, a binding such as `let save = cx.listener(Self::save);`
keeps GPUI's typed callback context. Extract widgets into typed components when
they have reusable structure; keep their retained entities and handles in their
owner. A dedicated formatter for macro bodies, similar to `leptosfmt`, is a
follow-up outside this plan.

## Paragraph and heading composition, wrapping and truncation

`p` and `h1`–`h6` collect text into one UTF-8 buffer with styled runs, rendered by GPUI's
text shaping system. End truncation measures the actual styled runs and font
fallbacks, and cuts only at grapheme boundaries. The inline tags are `span`, `strong` (bold), `em` (italic),
and `br` (newline). They do not create layout boxes or independent element IDs.
Nested formatting inherits and combines. Unquoted text preserves punctuation and
collapses whitespace between words to one space. Same-line spaces next to that
text are retained; leading/trailing multiline indentation is ignored. Gaps solely
between tags or braced expressions add no text: write `{" "}` when a separator is
needed. Quoted strings retain exact whitespace. HTML entities are not decoded.

Unquoted text must tokenize as Rust: use quoted strings for emoji, apostrophes,
arbitrary symbols, unmatched delimiters, or literal `<` and `{` characters.
Rust comments are ignored. `if`, `for` and `match` followed by their control-flow
syntax start control flow; quote such text when it should be displayed literally.
For example:

```rust
let name = "Ada";
let _ = rake::view! {
    <h2>Hello, <strong>{name}</strong>!</h2>
};
```

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

Paragraph and heading expressions must implement `AsRef<str>`, including `String`, `&str`,
`SharedString`, and borrowed string values. Use `format!` for numbers or other
`Display` values. Arbitrary `IntoElement` values, components, widgets, images,
and block tags are not paragraph content. `if`/`if let`, `for` and `match` work inside
paragraphs, evaluating only selected text once in written order. Inline loops
need no IDs because their output is text, not independently retained elements.
An empty `<p/>` is supported.

Inline `class` also accepts `text-[#rrggbb]` and `font-[Family_Name]`. Other supported inline utilities are `font-normal`, `font-medium`, `font-semibold`,
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

**Heading migration:** headings now accept string-like expressions and inline
`span`, `strong`, `em` and `br`, including conditional/loop text. Use `format!` for
numbers. Layout elements, components and embedded widgets belong in a `div` with
`role={gpui::accesskit::Role::Heading}` (set its level with
`style={|s| s.aria_level(2)}`). Explicit `aria-label` overrides the complete composed
heading name. For example, `<h2>"Count: "<strong>{count.to_string()}</strong></h2>`
forms one continuous text flow.

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
                on:click={|_, window, cx| window.dispatch_action(Box::new(Save), cx)}>
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
Literal-only buttons get labels automatically; headings use their full composed text. GPUI's
`focus_visible(|s| ...)` supplies keyboard-visible styling. Button contents should
be noninteractive; embed existing GPUI widgets for more complex controls.
Arbitrary Rust builder code inside `style` is an escape hatch and remains the
caller's responsibility, including any event listeners it installs directly.

## Conditions, loops and identity

Write `if`/`else if`/`else`, `for`, and `match` directly among children. Each branch or loop
body may contain zero or more siblings. `if let` and destructured loop patterns
work. Match arms accept patterns, optional guards and braced node lists; trailing
commas are optional. Rust checks exhaustiveness. Conditions and iterators are Rust expressions; wrap struct literals in
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

Match children work inside containers, paired components, paragraphs and headings:

```rust
# use rake::gpui::IntoElement;
# fn status(value: Option<&str>) -> impl IntoElement {
rake::view! {
    <h2>
        match value {
            Some(name) if !name.is_empty() => { "Hello, "<strong>{name}</strong> }
            _ => { <em>"Welcome"</em> },
        }
    </h2>
}
# }
```

Untaken branches/arms and empty-loop bodies do not evaluate their children. Built-in
layout tags add each child directly to their GPUI parent without an intermediate
tree or child vector. Paragraphs build their one text buffer and styled-run list. Paired component tags collect their authored children in the
component's `Vec<AnyElement>`. Large lists should use an embedded GPUI virtual
list instead of eagerly generating every item.

Each built-in layout tag and image gets a GPUI ID (SVGs are exempt), either explicit
or derived from its source site and position in the macro. GPUI scopes IDs under their nearest identified ancestor.
Outside composed paragraph/heading text, elements emitted directly by a loop
**must** have explicit IDs, including elements in conditional branches and match arms immediately inside that loop. Prefer stable data keys,
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
        header={<h2>Profile</h2>}
        footer={Some(rake::view! { <p>"Saved locally"</p> }.into_any_element())}>
        <p>"Your profile information."</p>
        <button on:click={|_, _, _| {}}>"Edit"</button>
    </Card>
};
```

A component property may contain one markup root, such as
`header={<h2>Profile</h2>}`. The macro expands that root and converts it to
`gpui::AnyElement`, so the property must accept `AnyElement`. This works with
built-in tags, custom components and retained `<view>` roots, including nested
markup properties. It adds no wrapper and evaluates the slot once in property
order. Ordinary Rust expressions keep their existing types; optional slots still
use `None` or `Some(rake::view! { ... }.into_any_element())`. Markup-valued
properties are component-only.

Properties map directly to Rust fields. Values have their declared types, with
ordinary Rust moves, borrows, and visibility checks. Rust rejects missing, unknown,
or incorrectly typed properties. Expression properties have no automatic `.into()`, cloning,
optional-property default, builder-method lookup, or property spreading. A quoted
value is an `&'static str`; use `label={"Title".into()}` for `SharedString`.
GPUI's `RenderOnce` requires `'static` components: own values or explicitly clone
retained handles instead of storing references to render-time locals.
Generic component types can be inferred from their fields; explicit generic
arguments in tag paths are not supported.

Self-closing tags initialize their listed properties, optionally followed by component defaults. Paired tags always
supply `children: Vec<AnyElement>`, including an empty vector for `<Stack></Stack>`.
This is a convention for arbitrary nested GPUI elements, not a `ParentElement`
requirement on the component struct. `if`, `for`, `match`, string-to-accessible-text
conversion, and retained `<view>` mounts work within those children. Alternatively,
pass `children={existing_children}` on a self-closing tag. Combining that property
with a paired tag is rejected. Authored properties evaluate once in written order,
followed by the selected children in render order. Component rendering happens
later through GPUI's usual `RenderOnce` lifecycle.

Named slots are ordinary typed properties, such as the required `header` and
optional `footer` above. Supply `None` for an absent optional slot, or provide it through a final `{..expr}`.
An optional final `{..expr}` uses ordinary Rust struct-update syntax:
`<Card {id} {..Default::default()}/>` or `<Card {id} {..base}>...</Card>`.
The component must provide the fields through that expression; no defaults are
implicitly created. Explicit fields override the base, and paired children
supply the children field. Defaults must be last and are component-only.
Authored fields evaluate first, then paired children, then the defaults expression,
following Rust's struct construction order. Normal move/borrow rules still apply.
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

## Diagnostics and editing

Independent local errors are reported together, including invalid attributes,
utilities and missing required properties. An invalid attribute value recovers at
the next attribute name or tag boundary. Malformed closing tags remain fatal.
On failure, valid parts of the invocation retain their GPUI builder context in
unreachable generated code, preserving expression and listener type checking for
editor tooling. Invalid component properties and invalid loop roots are omitted
from that recovery output to avoid unrelated inference/name errors. This recovery
code never runs, and erroneous markup never builds successfully.

## Current limits

Unknown tags, attributes, duplicate attributes, invalid utilities and malformed
markup are compile errors. This version does not implement input controls,
links/forms, automatic shortcut labels, selectable text or interactive inline
links, `aria-labelledby` and the complete ARIA vocabulary, general-purpose focus
traps outside the overlay controls, CSS selectors/cascade, HTML entity decoding,
fragments, explicit generic arguments in component tags, dedicated slot
syntax, automatic signal tracking, or markup hot reload. Inline text accepts a
fixed set of styles and rejects conditional classes, events and IDs. SVG leaves
use the documented restricted attribute set; wrap them for interaction and
identity. Existing GPUI components remain available through `{expression}`.
Raster images follow the application's asset/HTTP configuration, and SVG paths
use its GPUI asset source.

Automatic Rake inspection boundaries are not inserted. A returned styled element
can use the existing `.inspect(...)` extension; descendants need their own
boundaries to appear in Rake's Elements panel.

## Example and checks

```sh
cargo run -p rake-view
python3 scripts/build-web.py --example view
python3 scripts/serve-web.py --example view --port 8083

cargo test -p rake-macros --locked
cargo test -p rake --features experimental-view --test view_ui --locked
cargo test -p rake --features experimental-view --test view --test view_components --test view_paragraphs --locked
cargo test -p rake --features view-test-support --test view_state --locked
cargo test -p rake-view --features view-test-support --lib --locked
cargo test -p rake --features experimental-view --doc --locked
cargo check -p rake-view -p rake-inspector -p renamed-facade --all-targets --locked
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

Before heading composition, canonical events and the newer elements were added,
the example was smoke-tested on macOS: Enter and Command+I activation,
locked mouse/keyboard activation, Tab skipping disabled controls, native disabled
accessibility state, and row activation after a typed reorder. The component
follow-up also verified header/body/footer layout, dynamic Badge properties, and
retained row activation/focus after reordering inside Card. Those earlier macOS
checks also covered paragraph composition for nested bold/italic runs, dynamic
colored text, Unicode, single- and two-line ellipses, and complete untruncated
accessibility values.
The later events, tooltips, SVGs, semantic containers, rules, composed headings,
`match`, shorthand, defaults, unquoted text and markup-valued properties have compile, unit, UI and TestPlatform
frame coverage only, without native or browser interaction checks. Web compilation
is checked; browser input and screen-reader interaction have not been verified.

Expansion snapshots preserve builder order and generated identities. Compiler UI
fixtures cover individual and combined errors, attribute recovery, legacy event
warnings and expression/listener type checking beside macro errors. Utility
coverage tests compare the documented list with the checked table and compile all
supported methods against GPUI. Runtime construction tests cover match arms,
booleans, shorthand, component defaults, semantic roles and SVG labels. A direct
rust-analyzer completion probe verified typed listener fields beside an invalid
utility; it did not modify the source files.

The opt-in `view-test-support` feature enables GPUI's own deterministic TestPlatform.
Its frame tests exercise keyed element state, implicit button focus, retained
entity data/selection, explicit focus and scroll offsets across parent rerenders,
reordering and insertion. Component frame tests check forwarded root IDs, implicit
focus and keyed state across reorder/insertion, and retained child ownership.
Paragraph and heading frame tests cover shared inline flow, width changes, explicit
breaks, line limits, composed heading names and GPUI truncation. The TestPlatform
has no active accessibility adapter; heading node properties are inspected during
render alongside real layout measurements. Keyboard frame tests cover typed actions, Tab/Shift+Tab,
negative indices, consumed events, disabled mouse/keyboard activation, and
keyboard-visible styling. Pointer tests exercise right/middle buttons, hover and
wheel dispatch, including disabled suppression. The platform's window/text/GPU backend is a test double;
these checks do not establish native editor or browser input behavior.

The example's opt-in `view-test-support` tests render the actual counter,
components, retained history, popover, document dialog and help view. Reviewed
text fixtures assert the exact distinct strings sent to GPUI text layout, with
no whitespace normalization in the assertions. Scenarios cover initial and
incremented counts, locked/hidden history, open overlays and a selected virtual
document. The recorder keeps full composed text before truncation and omits
ellipsis measurement candidates; it does not assert pixel output, text order,
duplicate occurrences or native/browser rendering. CI runs these tests explicitly.

## Ambient theme runtime

With `experimental-view`, `rake::theme::install(cx, my_theme)` installs any
app-owned `'static` value and refreshes windows. `current::<MyTheme>()` clones
an `Rc`; `try_current::<MyTheme>()` returns `None` before installation. Different
theme types coexist. Observe replacements through the public
`ThemeGlobal<MyTheme>` GPUI global; change themes only through `install`.
Direct global writes leave the ambient value stale.

Themes are local to the UI thread: setup, switching and view construction must
run there, on native and Wasm. Existing built elements retain old values until
rebuilt. A missing theme panic names the type and tells you to call `install`.
`TextToken { size, line_height, weight }` describes paragraph typography using
pixels and an optional GPUI font weight. The class integration follows below.
