# HTML-like GPUI views

`rake::view!` is Rake's standard UI syntax, available with the default `gpui`
feature alongside themes, style presets and Base-backed controls. The macro
generates ordinary GPUI builders and Rust control flow. State stays in your GPUI
entities, handlers use GPUI events, and changes still need `cx.notify()`.
Rake is an alpha: APIs can change with migration notes; see [support](support.md).

`experimental-view` remains an empty, deprecated compatibility alias. Remove it
from existing manifests. Apps using `default-features = false` must explicitly
enable `gpui` for UI; the old alias no longer enables it. Headless builds remain
GPUI-free. `kit` and `devtools` remain separate opt-ins, and `view-test-support`
enables GPUI's test platform independently of the old alias.

For app-wide theme setup, register `.theme(colors)` on `App`, before or after `.routes(...)` or `.root(...)`. It installs on the UI thread once, before the initial view mounts, and page constructors don't reinstall it. Multiple theme types coexist, and the last registration wins for repeated types.

Use `rake::theme::install(cx, colors)` for later user-selected changes. See the [desktop example](../examples/desktop/src/main.rs).

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

Each invocation returns one concrete GPUI element or component and needs exactly one root tag. Closing tags must match, and empty elements can use `<div/>`. Rust comments work within markup.

Text can be:

- Unquoted: `Hello world`
- A Rust string literal: `"Hello"`
- A Rust expression: `{format!("Hello, {name}")}`

Literal text isn't interpolated automatically. Outside paragraphs and headings, expressions may contain statements and must produce an `IntoElement`. They can embed existing GPUI builders, `Entity` handles or `RenderOnce` components; clone retained handles explicitly when needed.

There's no string-based HTML parser, Dioxus runtime, virtual DOM or JavaScript execution.

Outside paragraphs and headings, literal strings and expressions whose GPUI element
is `SharedString` or a static string become identified, accessible `gpui::Text` nodes. Other embedded elements
keep their own identity and semantics. Literal-only buttons get an
accessible label automatically; supply `aria-label` for dynamic or icon buttons.
[Run `rake lint`](lint.md) to check accessible labels and positional loop IDs
without building the project. The linter and macro share `rake-view-syntax`.

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

SVGs accept `path`, `class`, conditional classes, `style`, `aria-label` and `aria-description`. For example, `<svg path="icons/search.svg" class="size-4" aria-label="Search" style={|s| s.text_color(rgb(0xffffff))}/>` uses GPUI's asset source and current text color.

SVGs have no macro ID and are exempt from loop-ID requirements, so wrap them in an identified element for retained identity, focus or events. SVGs and rules reject children. Style a rule's color explicitly, for example with `style={|s| s.bg(rgb(0x61717c))}`.

## Attributes, styling and events

Attribute values are quoted strings or `{Rust expressions}`.

- **Booleans:** bare `disabled`, `wrap`, `ellipsis`, `tab-navigation` and `aria-expanded` mean `={true}`. Component props can also be bare booleans. Other attributes require a value.
- **Shorthand:** `<div {id}/>` means `id={id}`. It keeps the property's normal type and ownership rules, so `{focus}` doesn't automatically borrow a handle.

Supported built-in attributes:

| Applies to | Attributes |
| --- | --- |
| Most elements | `id`, `class`, `style`, `aria-label`, `aria-description`, `tabindex`, `focus`, `scroll`, `key-context`, `tab-navigation`, `tooltip`, typed `on:Action` handlers |
| Containers and buttons | typed `role`, `aria-keyshortcuts`, `aria-expanded` |
| Buttons | `disabled` |
| Images | `src`, `alt` |
| SVGs | `path` |

`alt` maps to the image's accessible label, and `aria-label` can override it when supplied after `alt`. An empty `alt=""` marks the image as decorative and omits its accessibility role.

### Class grammar

A class string is a whitespace-separated sequence of checked GPUI utilities.
The macro resolves every token at compile time and emits builders in written
order. It supplies familiar markup syntax over GPUI's layout and interaction
model; it does not implement a stylesheet engine.

| Form | Meaning | Tiny example |
| --- | --- | --- |
| Fixed name | A checked GPUI convenience method | `flex-col`, `items-center` |
| `prefix-N` | Spacing: whole/half steps of 0.25rem; `border-N`: N px | `p-13`, `gap-1.5`, `border-3` |
| `text-` / `rounded-` | Named values, theme fields or explicit `[value]`; no numeric N | `text-sm`, `text-[11px]`, `rounded-[4px]` |
| `prefix-[value]` | Explicit finite value and unit | `w-[320px]`, `h-[50%]` |
| `prefix-A/B` | A supported relative fraction | `w-1/2`, `basis-1/3` |
| `-prefix-value` | Negative margin or inset | `-mt-1`, `-left-[4px]` |
| `text/bg/border-[#hex]` | Literal color | `text-[#abc]`, `bg-[#11223380]` |
| `font-[family]` | Font family; underscores become spaces | `font-[JetBrains_Mono]` |
| `variant:utility` | One interaction refinement | `hover:bg-surface` |
| `group/name` | Names an interaction group | `group/card` |
| `class:utility={bool}` | One conditional utility | `class:font-bold={selected}` |
| `class={if ...}` | Lazy selection between literal leaves | `if busy { "opacity-50" }` |
| Named theme value | Reads an app-owned theme field | `bg-surface`, `text-body` |
| `style={preset}` | Applies a typed reusable style function | `style={control}` |

A literal can be written as `class="..."` or as a literal leaf inside an
`if`/`if let`/`match` expression. Conditions and scrutinees run once between the
preceding and following authored attributes. Every leaf is checked, including
untaken ones. See [Themes](#themes) and [Style presets](#style-presets) for the
named-value and reusable-function APIs.

### Checked utility vocabulary

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
- `rounded-[4px]`, `border-[2px]`, `border-t-[2px]`, `border-3`, `border-t-1.5`.
- `min-w-[12px]`, `max-w-[50%]`, `min-h-[1rem]`, `max-h-[240px]`.
- `group/card`, `hover:bg-[#fff]`, `active:opacity-50`, `focus:border-2`.
- `focus-visible:text-[#000]`, `in-focus:font-bold`, `disabled:opacity-35`.
- `group-hover/card:bg-[#fff]`, `group-active/card:opacity-50`.

- `bg-surface`, `text-ink`, `border-line`, `font-ui`, `rounded-control`.

<!-- view-utilities:end -->

**Numbers.** Use plain decimal digits, optionally with a decimal point and more digits (`4`, `1.5`, `0.35`). Signs, exponents, `.5` and `4.` are rejected. Negative margins and insets use the utility's leading `-`.

| Utility | Accepts |
| --- | --- |
| Spacing and sizing `N` | Any nonnegative whole or half step, where one step is 0.25rem |
| `border-N`, `border-t/b/l/r-N` | Nonnegative pixel widths, including decimals (`border-1.5`) |
| `text-N`, `rounded-N` | Unsupported. Use a built-in name, theme field or bracketed length |
| `grid-cols-N` | 1–12 |
| Bracketed lengths | `px`, `rem` or `%`. Text sizes, border widths and radii accept only `px` and `rem` |
| Width, height, basis | Also fractions with denominators 2, 3, 4, 5, 6 and 12 |
| Negatives | Margins and insets only |
| Colors | `#rgb`, `#rrggbb` and `#rrggbbaa`, inside brackets |
| Opacity | 0–100 in steps of 5, or `[0..1]` |
| Font family | Underscores become spaces |

Existing scale values keep their GPUI convenience methods, and utilities map to the pinned GPUI builder methods. `truncate` and `whitespace-nowrap` are raw GPUI text styling. Composed paragraphs and headings also provide explicit `wrap`, `ellipsis` and `max-lines` controls.

Interaction variants use `hover:`, `active:`, `focus:`, `focus-visible:` and `in-focus:`. Named groups use `group/card`, `group-hover/card:` and `group-active/card:`.

- Each interaction kind merges into one GPUI style call, including from conditional class attributes.
- A named group must use the same name across all literal class branches, even if the leaves are mutually exclusive.
- Variants apply after authored attributes.
- `disabled:` is button-only and applies base styles at its written position.
- Variants can't stack, and they reject group and overflow-scroll utilities, because those configure interactivity.
- GPUI supports one named group per group-hover or group-active kind on an element.
- SVG interaction classes need a surrounding container.

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

Conditions, scrutinees and guards evaluate once in authored attribute order, and untaken nested conditions are skipped. Class conditions support let chains such as `if ready && let Some(_) = item && enabled { "p-1" } else { "p-2" }`. Checks run left to right and stop on the first failure, and each pattern inspects a borrowed scrutinee. A missing `else` means no classes.

**Patterns.** Use `Some(_)`, `_`, uppercase unit variants or constants such as `None`, or qualified paths such as `Mode::Dark`. Lowercase bindings and `ref`, `mut` or `@` patterns are rejected. rustc checks match exhaustiveness.

Variables, function calls and `format!` can't produce class strings. Inline text accepts the same literal-branch selection with its restricted run utilities, but interaction variants aren't supported inside runs.

Errors quote each bad class and its ordinal position. Stable Rust spans
underline the whole literal, so the message identifies the individual token:

```text
2nd class `justify-betwen`: unsupported utility class `justify-betwen`; did you mean `justify-between`?
3rd class `hovr:p-2`: unknown variant `hovr:`; did you mean `hover:`?
1st class `w-[320]`: arbitrary lengths require a unit; add px, rem or %
1st class `opacity-[1.2]`: opacity must be in the range 0..=1
```

All bad classes are reported together, including errors across branch leaves.
Names under theme prefixes go to Rust's field/type checker; `bg-bra` reports
`no field bra` on your theme, rather than a utility typo suggestion.

Conditional classes use a literal utility name: `class:shrink-0={fixed}` or
`class:font-bold={selected}`. Each condition is evaluated once, and the utility
applies in attribute order only when true. A negative utility can follow a
variant prefix: `class:hover:-mt-1={raised}`; `class:-mt-1={raised}` also works.
Unknown/dynamic names are errors.
Inline text tags reject conditional classes; select a `HighlightStyle` through
`style={...}` there instead.

`style={|s| ...}` receives the GPUI builder through `FluentBuilder::map` and can use its typed styling API, including colors, dimensions, hover and focus. It should return the same builder type and preserve the element's identity. Other dynamic styles belong here; CSS strings and external stylesheets are unsupported.

Order of application:

1. IDs and an image's source are configured first.
2. A button's `disabled` predicate is evaluated once, before its builder.
3. Classes and style closures apply in attribute order, so later settings win.
4. Focus and tab-index configuration and the optional Tab handler apply after authored styles and handlers, regardless of attribute order.

Other attribute expressions run once, before children, in written order. Macro-authored button event handlers are only constructed when enabled.

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

Run `rake format` after editing Rust or `view!` markup. See
[Formatting Rust and view markup](format.md) for layout rules, preservation
guarantees, check mode, and rust-analyzer setup.

## Paragraph and heading composition, wrapping and truncation

`p` and `h1`–`h6` collect text into one UTF-8 buffer with styled runs, rendered by GPUI's text shaping. End truncation measures the actual runs and font fallbacks, and cuts only at grapheme boundaries.

The inline tags are `span`, `strong` (bold), `em` (italic) and `br` (newline). They don't create layout boxes or independent element IDs, and nested formatting combines.

Whitespace rules:

- Unquoted text keeps punctuation and collapses whitespace between words to one space.
- Same-line spaces next to that text are kept. Leading and trailing multiline indentation is ignored.
- Gaps solely between tags or braced expressions add no text, so write `{" "}` when you need a separator.
- Quoted strings keep exact whitespace.

HTML entities aren't decoded.

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

Paragraph and heading expressions must implement `AsRef<str>`, including `String`, `&str`, `SharedString` and borrowed strings. Use `format!` for numbers and other `Display` values. Arbitrary `IntoElement` values, components, widgets, images and block tags aren't paragraph content.

`if`, `if let`, `for` and `match` work inside paragraphs, evaluating only selected text once in written order. Inline loops need no IDs, because their output is text rather than retained elements. An empty `<p/>` is supported.

Inline `class` accepts `text-[#rrggbb]`, `font-[Family_Name]`, `font-normal`, `font-medium`, `font-semibold`, `font-bold`, `italic`, `not-italic` and `underline`.

Inline `style` takes a typed `gpui::HighlightStyle` value, **not** the builder closure used on layout tags. It supports run properties such as color, weight, italics, underline, strikethrough and background. `font-family` accepts a GPUI font-family string. Attributes apply after the tag's default style, in written order.

Font size, line height, alignment, dimensions, padding and other layout belong on `p`. Inline IDs, events and accessibility overrides are rejected, and `br` accepts no attributes or children.

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

Wrapping and truncation need a constrained width.

| Controls | Result |
| --- | --- |
| `ellipsis` with `max-lines` | Multiline ellipsis |
| `ellipsis` only | One visible line, including when the source has explicit breaks |
| `max-lines` only | Clips without an ellipsis |
| `ellipsis={false}` | No truncation styling |

These controls use the normal attribute order, so later raw GPUI styling can override them. Only end truncation is supported, and text transforms through raw style builders aren't implemented. Selection and interactive links aren't implemented either.

The paragraph keeps one accessible text node with the full text, even when the visual preview is truncated.

**Heading migration:** headings now accept string-like expressions and inline `span`, `strong`, `em` and `br`, including conditional and loop text. Use `format!` for numbers. For example, `<h2>"Count: "<strong>{count.to_string()}</strong></h2>` forms one continuous text flow.

Layout elements, components and embedded widgets belong in a `div` with `role={gpui::accesskit::Role::Heading}`; set its level with `style={|s| s.aria_level(2)}`. An explicit `aria-label` overrides the whole composed heading name.

**Migration:** `p` previously accepted arbitrary layout children and embedded GPUI elements. Move those into a `div` or `section`, or embed an existing custom text component there.

Use inline `span` styles only for text inside `p`; standalone spans keep their layout API. Paragraph IDs still scope under their parent and follow normal loop identity rules.

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

`on:Save` and qualified paths such as `on:crate::actions::Save` expand to typed GPUI `on_action::<Save>` listeners. Action types must implement `gpui::Action`, and handlers use the normal GPUI signature and accept `cx.listener(Self::save)`.

Register shortcuts once during application setup:

- `KeyBinding::new` takes static bindings with explicit modifiers. Use `"cmd-s"` when `cx.platform_info().os == gpui::OperatingSystem::Mac` and `"ctrl-s"` otherwise.
- For dynamic strings or the `secondary-` alias, use `KeyBinding::load` with the app context. `Keystroke::parse_with_os` is the matching input parser. Both use the host OS, on Web too.

`key-context` supplies GPUI's keymap context on the focused dispatch path, and doesn't register shortcuts. `aria-keyshortcuts` is accessibility metadata only, so supply the actual platform shortcut yourself.

`tab-navigation={true}` installs a bubbling handler for unmodified Tab and Shift+Tab. It calls GPUI's window-wide `focus_next` or `focus_prev`, then prevents the default and stops propagation.

- A descendant or earlier handler can consume the event with `cx.stop_propagation()` or `window.prevent_default()`, so nested handlers advance only once.
- It's opt-in, and isn't a focus trap or limited to the container's subtree.
- Set initial window focus once, so the handler is on the keyboard dispatch path.

**Tab order.** Buttons get a retained focus handle and `tabindex={0}` by default, including when `focus` supplies a stored handle. Use `tabindex={-1}` to skip sequential navigation while allowing explicit focus. Nonnegative indices use GPUI's ordering. On other elements, `tabindex` establishes a tab stop and can configure a supplied handle.

**Handles.** `focus={&self.focus}` tracks a stored `FocusHandle`, and `scroll={&self.scroll}` tracks a stored `ScrollHandle`. Construct them once in the owning view, not in `render`, and configure overflow styling for scrolling. Both need shared borrows of stored variables, fields or indexed entries; temporaries and method-call results are compile errors. Focus attributes take precedence over raw focus configuration in `style`.

`disabled={condition}` works on `button` only. Disabled buttons:

- Skip Tab and don't take mouse focus.
- Suppress macro-authored event and action handlers.
- Block mouse and unmodified Enter or Space activation.
- Expose AccessKit's disabled flag without activation or focus actions.

A button disabled while focused keeps its focus anchor, so Tab and enclosing actions continue to work. It isn't automatically blurred. Action handlers must still check whether their operation is available, because a shortcut or other code can invoke an enclosing handler. Disabled appearance stays explicit, for example an opacity style.

- Use `aria-label` for dynamic or icon controls, and `aria-description` for extra context.
- Use `aria-expanded={open}` on disclosure buttons.
- Use typed `role={gpui::accesskit::Role::Group}` for containers with custom semantics.
- Literal-only buttons get labels automatically, and headings use their full composed text.
- GPUI's `focus_visible(|s| ...)` supplies keyboard-visible styling.

Button contents should be noninteractive, so embed GPUI widgets for more complex controls. Builder code inside `style` is an escape hatch and remains your responsibility, including any event listeners it installs.

## Conditions, loops and identity

Write `if`/`else if`/`else`, `for` and `match` directly among children. Each branch or loop body may contain zero or more siblings. `if let` and destructured loop patterns work.

Match arms accept patterns, optional guards and braced node lists, and trailing commas are optional. Conditions and iterators are Rust expressions, so wrap struct literals in parentheses as in ordinary Rust.

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

Untaken branches and empty-loop bodies don't evaluate their children. Built-in layout tags add each child directly to their GPUI parent, with no intermediate tree. Paragraphs build one text buffer and run list. Paired component tags collect their children in the component's `Vec<AnyElement>`.

For large lists, embed a GPUI virtual list instead of eagerly generating every item.

Each built-in layout tag and image gets a GPUI ID (SVGs are exempt), either explicit or derived from its source site and position. GPUI scopes IDs under their nearest identified ancestor.

**Elements emitted directly by a loop must have explicit IDs**, outside composed paragraph and heading text. This includes elements in conditional branches and match arms immediately inside that loop.

- Prefer stable data keys, such as `id={("row", row.id)}`, over positions when rows can reorder.
- Nested children under an identified row can use generated IDs.
- Direct text or expression children in loops need a wrapper with a stable ID.
- Embedded non-string expressions own their identity, including elements from nested macro invocations.
- Reusable helper views rendered as siblings must accept distinct root IDs.

Debug builds check the actual GPUI IDs of direct children added to each macro-authored parent, including from loops, taken conditions and embedded GPUI roots. A duplicate panics before layout with the ID and both source sites. IDs can repeat under different identified parents, and untaken branches aren't checked. Release builds omit these checks.

Limits of the check:

- It's local to direct child roots. Opaque GPUI descendants and parents built outside the macro aren't enumerated.
- A `RenderOnce` component is opaque until rendered, so its root ID can't be checked at construction. Its authored child vector is checked first. Components must forward unique IDs to their roots.
- Stable key values remain your responsibility.

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

A component property may contain one markup root, such as `header={<h2>Profile</h2>}`. The macro expands it and converts it to `gpui::AnyElement`, so the property must accept `AnyElement`.

This works with built-in tags, custom components, retained `<view>` roots and nested markup properties. It adds no wrapper and evaluates the slot once in property order. Ordinary Rust expressions keep their types, and optional slots still use `None` or `Some(rake::view! { ... }.into_any_element())`. Markup-valued properties are component-only.

Properties map directly to Rust fields, with ordinary moves, borrows and visibility checks. Rust rejects missing, unknown or incorrectly typed properties.

- There's no automatic `.into()`, cloning, optional-property default, builder-method lookup or property spreading.
- A quoted value is an `&'static str`. Use `label={"Title".into()}` for `SharedString`.
- GPUI's `RenderOnce` requires `'static` components, so own values or explicitly clone retained handles rather than storing references to render-time locals.
- Generic component types can be inferred from fields. Explicit generic arguments in tag paths aren't supported.

Self-closing tags initialize their listed properties, optionally followed by component defaults. Paired tags always supply `children: Vec<AnyElement>`, including an empty vector for `<Stack></Stack>`. That's a convention, not a `ParentElement` requirement on the struct.

`if`, `for`, `match`, string-to-accessible-text conversion and `<view>` mounts work within those children. You can instead pass `children={existing_children}` on a self-closing tag, but not on a paired tag.

Authored properties evaluate once in written order, followed by the selected children in render order. Component rendering happens later through GPUI's usual `RenderOnce` lifecycle.

**Slots.** Named slots are ordinary typed properties, such as the required `header` and optional `footer` above. Supply `None` for an absent optional slot, or provide it through a final `{..expr}`. The component chooses where to render each slot and its children. Dedicated `<slot>` syntax isn't implemented.

**Defaults.** A final `{..expr}` uses Rust struct-update syntax: `<Card {id} {..Default::default()}/>` or `<Card {id} {..base}>...</Card>`.

- The component must provide the fields through that expression; no defaults are created implicitly.
- Explicit fields override the base, and paired children supply the children field.
- It must come last and is component-only.
- Authored fields evaluate first, then paired children, then the defaults expression. Normal move and borrow rules apply.

**Callbacks** work the same way: `on_save={cx.listener(Self::save)}` must match the field's Rust type.

**Names.** Component properties must be Rust field names, with `r#` for keywords. Namespaced `on:Action` and hyphenated attributes such as `aria-label` don't apply to component tags. Even `class`, `style`, `disabled`, `focus` and `id` are ordinary fields, and the component decides how to use them.

Forward a component's `id: ElementId` property to an identified GPUI root, as `Card` does above. Component tags don't insert layout wrappers, generate IDs or create retained state.

Direct component roots in loops need an explicit `id` with stable data keys. Keep repeated instances' root IDs unique, including outside loops. Changing or discarding those roots has the same state and focus consequences as ordinary GPUI rendering.

Component structs are reconstructed when their parent renders. Keep persistent data and stateful widgets in caller-owned entities, and mount them with `<view entity={&self.editor}/>` inside component content.

The macro constructs the component value, and never calls `cx.new` or retains it in a cache. Your property expressions and `RenderOnce` implementations remain normal Rust and can allocate if you choose.

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

`view` clones the borrowed entity handle, keeping its `EntityId`. It never calls an initializer or creates an entity.

A direct `view` in a loop uses that entity's own stable GPUI ID and needs no separate `id`. Reordering the same entities under one parent keeps their identity. Mounting the same entity twice under one parent is a duplicate-ID error in debug builds.

The `entity` value must borrow a stored place, such as `&self.editor`, `&row.editor` or `&self.editors[index]`. `&cx.new(...)` and `self.editor.clone()` are rejected with an ownership diagnostic.

`view` accepts only `entity`, so put styles and identity scopes on a surrounding element. `{expression}` embedding remains available, and you're responsible for any allocations or entity construction in it.

Owned entity data, including editor contents and selection, stays with that entity. Stored focus and scroll handles keep their identity, and keyed mounted elements reuse GPUI's element state.

- Changing an ID or identified ancestor, moving a component to another parent, or unmounting it can reset implicit element state.
- Unmounting can blur focus. The macro doesn't focus hidden components or restore focus automatically.
- An entity kept by its owner retains its data while absent from the tree. Dropping or replacing the owner's handle ends its lifetime normally.

There's no global component cache or extra state runtime.

## Scrolling, virtual lists and overlays

Retained scroll handles work through `scroll`, and GPUI virtual lists embed through `{expression}`.

The `rake::overlay` module adds `OverlayHost`, `OverlayAnchor`, `Popover` and `Dialog` as typed components, with retained state, live anchor geometry, stacked dismissal and modal focus containment. See [scrolling and overlays](../crates/rake/overlays.md). The example opens a retained 10,000-document virtual list from a nested dialog.

## Diagnostics and editing

Independent local errors are reported together, including invalid attributes, utilities and missing required properties. An invalid attribute value recovers at the next attribute name or tag boundary, while malformed closing tags remain fatal.

On failure, valid parts keep their GPUI builder context in unreachable generated code, so editor tooling still type-checks expressions and listeners. Invalid component properties and loop roots are omitted to avoid unrelated errors. This code never runs, and erroneous markup never builds.

## Current limits

Unknown tags, attributes, duplicate attributes, invalid utilities and malformed markup are compile errors.

Not implemented in this version:

- Input controls, links and forms.
- Automatic shortcut labels, selectable text and interactive inline links.
- `aria-labelledby` and the complete ARIA vocabulary.
- General focus traps outside the overlay controls.
- CSS selectors or cascade, and HTML entity decoding.
- Fragments, explicit generic arguments in component tags and dedicated slot syntax.
- Automatic signal tracking and markup hot reload.

Inline text accepts checked text-run styles and theme colors and fonts, and rejects conditional class attributes, events and IDs. Literal class branches are supported. SVG leaves use their restricted attribute set, so wrap them for interaction and identity. Existing GPUI components remain available through `{expression}`. Raster images follow your asset and HTTP configuration, and SVG paths use the GPUI asset source.

Every rendered macro-authored element, component or entity mount, and ordinary text or expression child automatically creates a development inspection boundary, so no `.inspect(...)` calls are needed. An existing `.inspect("name")` on a built-in root renames that boundary without adding another.

GPUI IDs, layout and accessibility still delegate to the original element. Generated site IDs and explicit loop keys provide identities across frames, and removal expires them. Repeated unkeyed sites remain ambiguous.

Components keep their typed properties through a transparent construction wrapper. Use `.into_inner()` when you need the original component or concrete GPUI root. That removes only that boundary, leaving instrumented descendants.

Component `id` properties using `ElementId`, strings, common integer keys, common GPUI named-integer tuples or UUIDs supply inspection keys without changing GPUI IDs. Other types don't supply a key, so repeated sites have the usual ambiguity.

The Source pane lists authored tags and style-related attributes with file, line, column and bounded Rust expression spellings. It records declarations, not selected conditional branches or methods hidden in style closures.

Inline paragraph tags are text runs, so their style metadata belongs to the paragraph, with no invented per-run boxes. Their composed text appears as a captured text child, including styling runs, line breaks and dynamic values. Ordinary text children in buttons, headings, labels and other containers are captured too. Text is bounded to 16 KiB per boundary, and editable control values stay excluded.

Capture remains opt-in through the routed or standalone `App::root` development host, and count/byte limits
still apply. Without an inspection host, wrappers do not sample or serialize styles.
Embedded third-party component internals and reused cached GPUI subtrees remain
opaque unless they execute their own instrumented rendering. See
[element inspection](element-inspection.md) for coverage and limits.

## Example and checks

```sh
cargo run -p rake-view
python3 scripts/build-web.py --example view
python3 crates/rake-cli/scripts/serve-web.py --example view --port 8083

cargo test -p rake-macros --locked
cargo test -p rake-app --test view_ui --locked
cargo test -p rake-app --test view --test view_components --test view_paragraphs --locked
cargo test -p rake-app --features view-test-support --test view_state --locked
cargo test -p rake-view --features view-test-support --lib --locked
cargo test -p rake-app --doc --locked
cargo check -p rake-view -p renamed-facade --all-targets --locked
cargo check -p rake-view -p renamed-facade --lib --target wasm32-unknown-unknown --locked
```

The example uses reusable `components::Card` and `components::Badge`. Card accepts typed header and footer slots and collects the counter controls and retained history into its body. Badge derives its label and appearance from typed properties. Paragraphs show nested bold and italic runs, colored dynamic text, Unicode, a two-line preview and a single-line ellipsis. Native and Web use the same render method.

| Key | Action |
| --- | --- |
| Command+I on macOS, Control+I elsewhere | Increment |
| Escape | Reset |
| H | Toggle history |
| R | Reverse rows |
| L | Lock or unlock editing |
| T | Switch the ambient theme |
| B, or Enter/Space on a focused row | Activate a row |

Reset and Reverse are disabled when unavailable, and locking disables Increment and Reset. Tab and Shift+Tab skip disabled controls and show focus styling. History rows are retained entities with their own focus and click counts, and the list uses a stored scroll handle. Resetting or hiding history returns focus to the counter root before rows disappear.

Browser input and screen-reader interaction haven't been verified. Several newer elements have automated coverage only, without native or browser interaction checks. See [test coverage](internal/test-coverage.md).

The opt-in `view-test-support` feature enables GPUI's deterministic TestPlatform for your own frame tests. Its window, text and GPU backend is a test double, so it doesn't establish native editor or browser input behavior.

## Themes

With the default `gpui` feature, `rake::theme::install(cx, my_theme)` installs any
app-owned `'static` value and refreshes windows. `current::<MyTheme>()` clones
an `Rc`; `try_current::<MyTheme>()` returns `None` before installation. Different
theme types coexist. Observe replacements through the public
`ThemeGlobal<MyTheme>` GPUI global; change themes only through `install`.
Direct global writes leave the ambient value stale.

Themes are local to the UI thread, so setup, switching and view construction must run there, on native and Wasm. Existing built elements keep old values until rebuilt. A missing-theme panic names the type and tells you to call `install`.

`TextToken { size, line_height, weight }` describes paragraph typography using pixels and an optional GPUI font weight. Use it with a named `text-` class on the paragraph.

### Declaration and field types

Declare your type once at the crate root, and install it on the UI thread:

```rust
use rake::gpui::{self, rgb};
#[derive(Clone, rake::Theme)]
struct Colors { surface: gpui::Rgba, ink: gpui::Rgba, hover: gpui::Rgba }
rake::theme!(Colors);
fn setup(cx: &mut gpui::App) {
    rake::theme::install(cx, Colors { surface: rgb(0x18232b), ink: rgb(0xf0f4e8), hover: rgb(0x365849) });
}
fn panel() -> impl gpui::IntoElement {
    rake::view! { <div class="bg-surface text-ink hover:bg-hover"><p><span class="text-ink">Themed</span></p></div> }
}
```

Unknown names under `text-`, `bg-`, `border-`, `font-` and `rounded-` read fields
from this type, after built-in values have been checked. Hyphens become
underscores (`bg-surface-raised` reads `surface_raised`). Derive `Theme` checks
reserved built-in names and requires `Clone`; fields must be named. `hover` is
a valid field name. There is no built-in palette or `dark:` variant.

| Prefix | Accepts |
| --- | --- |
| `text-` | GPUI colors or `TextToken` |
| `bg-` | GPUI fills |
| `border-` | Colors |
| `font-` | `SharedString` or `&'static str` |
| `rounded-` | Pixels or rems |

Inline `text-` accepts colors only: a `TextToken` gives a `ThemeRunText` trait error, because font size belongs on `p`. Inline `font-` is also supported.

Wrong field names and value types produce Rust field or trait errors, with spans on the class literal. A missing declaration reports “cannot find type `__RakeTheme` in the crate root”, so add `rake::theme!(YourTheme)` at that root.

Each invocation clones one ambient `Rc` before building its elements, including
variant refinements. Nested `view!` expressions bind their own handle. Deep
components need no theme props. Switching calls `install` with another value
of the same type and rebuilds views; existing styles retain their old values.
`examples/view` demonstrates a typed `ToggleTheme` action and a T shortcut.

## Text inputs

With the default `gpui` feature, import `Input`, `InputState`, `Textarea`,
`TextareaState`, and `InputEvent` from `rake::ui::input`. These are the GPUI Base
types and follow Base's versioning. Rake initializes Base during app setup.

Create the state entities once in the owning view. Pass references to those
entities when rendering the controls.

```rust,ignore
use rake::gpui::AppContext as _;
use rake::ui::input::{Input, InputState, Textarea, TextareaState};

let message = cx.new(|cx| InputState::new(window, cx).placeholder("Message"));
let notes = cx.new(|cx| TextareaState::new(window, cx).placeholder("Notes"));

// In the owning view's render method.
rake::view! {
    <div class="flex flex-col gap-2">
        {Input::new(&self.message)}
        {Textarea::new(&self.notes)}
    </div>
}
```

Use `InputState::masked(true)` for a masked field. Subscribe to
`InputEvent::Change` and retain the subscription if the owning view needs to
react to edits. The state supplies the editing behavior and focus handle.
Style the surrounding elements for the layout you want.

Run `cargo run -p rake-view --locked` and select **Try text inputs** to try
single-line, masked, and multiline fields. The example displays message and
note counts and whether the masked field is filled. Its labeled inspection
boundaries expose focus and geometry for agent actions. CDP DOM, accessibility,
and semantic snapshots omit the live input values. Storage inspection reads
explicitly registered stores.

## Style presets

Define reusable, typed builder functions using the same class grammar:

```rust
#[rake::style("h-[24px] px-2 rounded-sm hover:opacity-50")]
pub fn control();
fn toolbar() -> impl rake::gpui::IntoElement {
    rake::view! { <button style={control}>Save</button> }
}
```

Use `style={control}` or compose `style={|s| control(s).w(gpui::px(10.))}`. Presets support literal utilities, themes and merged variants.

The generated function accepts `Styled`. Hover, focus and group utilities add `InteractiveElement`, and active and scrolling utilities require `StatefulInteractiveElement`. A plain preset also works on `StyleRefinement`.

Marker functions have no body, parameters, generics or qualifiers, and visibility and doc attributes are preserved. Conditional attributes, branches and button-only `disabled:` need an element context and are rejected.

A preset's `hover:` conflicts with `hover:` classes on the same element, as do
other interaction kinds. The token scanner cannot inspect a named function;
GPUI's debug assertion is the backstop. Keep each kind in one place.

The inspector also declares an app-owned `Palette` theme using the same grammar, literal branches and focus preset, and `style={control}` applies the shared preset to its toolbar buttons.

### Shared semantic colors

`rake::theme::Semantic` is a flat palette with Base's 18 color names and an `appearance` field. Declare `rake::theme!(rake::theme::Semantic)` at the crate root to use `bg-background`, `text-foreground`, `bg-primary`, `text-primary-foreground`, `bg-surface` and the other semantic roles. Install `Semantic::light()` or `Semantic::dark()` through `rake::theme::install(cx, palette)`.

Application-defined themes stay independent and work unchanged. Typography, spacing and shadow scales remain separate Base settings.

Installation updates the ambient Rake copy and Base's colors. Rake observes the
actual Base global effect queue and replaces its copy only when values change,
so an unchanged notification cannot create an update loop.

With the optional native `kit` feature, installation uses Kit's `Theme::update`
and its Base projection. Changes made by Kit itself refresh Rake's ambient copy
through the Base-global observer. Base `surface` means Kit `popover`, and Base
`destructive` means Kit `danger`. See [Base and Kit](gpui-kit.md) for the shared
Root, native-only Kit gate, and inspection boundaries.
