Builds GPUI views from HTML-like markup.
With `devtools`, nodes expose bounded inspection and authored style sources.
Without it, inspection wrappers and source descriptors are removed at expansion.

```
let rows = ["One", "Two"];
let _view = rake::view! {
    <div class="flex flex-col gap-2 p-4">
        <h1>"Example"</h1>
        if rows.is_empty() {
            <p>"No rows"</p>
        } else {
            for (index, row) in rows.iter().enumerate() {
                <p id={("row", index)}>{row.to_string()}</p>
            }
        }
    </div>
};
```

Classes support explicit units, interaction variants and literal branches:

```
let selected = true;
let _ = rake::view! {
    <button class="h-[24px] px-2 hover:opacity-50"
        class:font-bold={selected}>Save</button>
};
```

App-owned themes and reusable style functions keep views concise:

```
use rake::gpui::{self, IntoElement};
#[derive(Clone, rake::Theme)]
struct Colors { surface: gpui::Rgba, ink: gpui::Rgba }
rake::theme!(Colors);
#[rake::style("px-2 rounded-sm hover:bg-surface")]
fn control();
fn panel() -> impl IntoElement {
    rake::view! { <button class="text-ink" style={control}>Save</button> }
}
fn setup(cx: &mut gpui::App) {
    rake::theme::install(cx, Colors { surface: gpui::rgb(0x223344), ink: gpui::rgb(0xffffff) });
}
# fn main() {}
```

Declare the theme at the crate root and install it on the UI thread before
constructing views. Built-in utility values take precedence over theme fields;
hyphens in field names become underscores. Inline theme text accepts colors,
while `TextToken` typography belongs on the paragraph. Each invocation reads
one theme handle at construction. Presets and classes cannot both set the same
interaction kind; named functions are opaque to the conflict heuristic.

Mount stateful views by borrowing an entity retained by their owner:

```
# use rake::gpui::{Entity, IntoElement, Render};
fn mounted<V: Render>(editor: &Entity<V>) -> impl IntoElement {
    rake::view! { <view entity={&editor}/> }
}
```

Typed actions, scoped keymaps, and opt-in Tab navigation use GPUI directly:

```
use rake::gpui::{self, FocusHandle, IntoElement};
gpui::actions!(editor, [Save]);
fn toolbar(focus: &FocusHandle, busy: bool, cx: &mut gpui::App) -> impl IntoElement {
    rake::view! {
        <section focus={&focus} key-context="Editor" tab-navigation={true}
            on:Save={|_: &Save, _, _| {}}>
            <button disabled={busy} aria-keyshortcuts="Control+S"
                on:click={|_, window, cx| window.dispatch_action(Box::new(Save), cx)}>
                "Save"
            </button>
        </section>
    }
}
```

Register key bindings once during application setup. Disabled buttons skip Tab
and suppress activation; enclosing action handlers must check availability too.
A focused button retains its focus anchor when disabled so Tab can leave it.

Action attributes require real GPUI action types:

```compile_fail
struct NotAnAction;
let _ = rake::view! { <div on:NotAnAction={|_, _, _| {}}/> };
```

Uppercase tags construct typed GPUI components. Paired tags supply a children
vector; named slots can be ordinary properties such as `header: AnyElement`.

```
use rake::gpui::{self, prelude::*};
#[derive(IntoElement, rake::Props)]
struct Stack {
    #[prop(no_into)]
    id: gpui::ElementId,
    header: gpui::AnyElement,
    children: Vec<gpui::AnyElement>,
}
impl RenderOnce for Stack {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        gpui::div().id(self.id).flex().flex_col()
            .child(self.header).children(self.children)
    }
}
# fn example(window: &mut gpui::Window, cx: &mut gpui::App) {
let _stack = rake::view! {
    <Stack id={"stack".into()} header={<h2>Typed children</h2>}>
        <p>Ordinary GPUI elements.</p>
    </Stack>
};
# }
```

Component properties follow Rust field types, visibility, and ownership rules.
Markup-valued properties such as `header={<h2>Title</h2>}` produce `AnyElement`.
Without `experimental-reactive`, expression properties retain their types without implicit conversions. With the feature enabled, derived props builders accept `Into` unless a field uses `#[prop(no_into)]`. Component
root IDs must be forwarded to the rendered GPUI root. Retain state in caller-owned entities.

```compile_fail
struct Badge { label: String }
let _ = rake::view! { <Badge label={42}/> };
```

```compile_fail
struct Badge { label: String, highlighted: bool }
let _ = rake::view! { <Badge label={"Label".into()}/> };
```

```compile_fail
struct Badge { label: String }
let _ = rake::view! { <Badge label={"Label".into()} unknown={true}/> };
```

```compile_fail
struct Badge { label: String }
let _ = rake::view! { <Badge label={"Label".into()}>"No children field"</Badge> };
```

Paragraphs compose string-like expressions and inline styles into one text layout:

```
use rake::gpui;
let labels = ["Ada", "Lin"];
let _paragraph = rake::view! {
    <p wrap={true} max-lines={2} ellipsis={true}
        style={|s| s.w(gpui::px(240.))}>
        <strong>"Hello, "</strong>
        for (index, label) in labels.iter().enumerate() {
            if index > 0 { ", " }
            <span class="italic underline">{label}</span>
        }
        <br/>"One shaped paragraph."
    </p>
};
```

Inline `style` accepts `gpui::HighlightStyle`; sizes and layout belong on `p`.
Paragraphs preserve the complete accessible text when visually truncated.
`max-lines={0}` normalizes to one. Selection and inline links are not implemented.
Use `div`/`section` for layout and embedded widgets; paragraph expressions must
implement `AsRef<str>`:

```compile_fail
let _ = rake::view! { <p>{rake::gpui::div()}</p> };
```

```compile_fail
let _ = rake::view! { <p><button>"A widget"</button></p> };
```
