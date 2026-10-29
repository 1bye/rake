//! Rake's portable application foundation.
//!
//! Default builds include the pinned GPUI native/browser adapters. Disable default
//! features for HTTP, data and diagnostics consumers that do not need a UI.

extern crate self as rake;

pub mod control;

pub use rake_diagnostics as diagnostics;

pub use rake_http as http;

pub mod inspection;
pub mod quit;
pub mod reload;
pub mod router;
pub mod services;
pub mod tasks;

#[cfg(feature = "gpui")]
mod application;

#[cfg(feature = "experimental-view")]
use rake_ui::view;

#[cfg(feature = "experimental-view")]
pub mod overlay;

#[cfg(feature = "experimental-view")]
pub use rake_ui::theme;

#[cfg(feature = "gpui")]
pub use application::{App, AppError};
#[cfg(feature = "gpui")]
pub use gpui;
pub use rake_macros::{Command, Route, app, data, instrument};
#[cfg(feature = "experimental-view")]
/// Builds GPUI views from experimental HTML-like markup.
///
/// ```
/// let rows = ["One", "Two"];
/// let _view = rake::view! {
///     <div class="flex flex-col gap-2 p-4">
///         <h1>"Example"</h1>
///         if rows.is_empty() {
///             <p>"No rows"</p>
///         } else {
///             for (index, row) in rows.iter().enumerate() {
///                 <p id={("row", index)}>{row.to_string()}</p>
///             }
///         }
///     </div>
/// };
/// ```
///
/// Classes support explicit units, interaction variants and literal branches:
///
/// ```
/// let selected = true;
/// let _ = rake::view! {
///     <button class="h-[24px] px-2 hover:opacity-50"
///         class:font-bold={selected}>Save</button>
/// };
/// ```
///
/// App-owned themes and reusable style functions keep views concise:
///
/// ```
/// use rake::gpui::{self, IntoElement};
/// #[derive(Clone, rake::Theme)]
/// struct Colors { surface: gpui::Rgba, ink: gpui::Rgba }
/// rake::theme!(Colors);
/// #[rake::style("px-2 rounded-sm hover:bg-surface")]
/// fn control();
/// fn panel() -> impl IntoElement {
///     rake::view! { <button class="text-ink" style={control}>Save</button> }
/// }
/// fn setup(cx: &mut gpui::App) {
///     rake::theme::install(cx, Colors { surface: gpui::rgb(0x223344), ink: gpui::rgb(0xffffff) });
/// }
/// # fn main() {}
/// ```
///
/// Declare the theme at the crate root and install it on the UI thread before
/// constructing views. Built-in utility values take precedence over theme fields;
/// hyphens in field names become underscores. Inline theme text accepts colors,
/// while `TextToken` typography belongs on the paragraph. Each invocation reads
/// one theme handle at construction. Presets and classes cannot both set the same
/// interaction kind; named functions are opaque to the conflict heuristic.
///
/// Mount stateful views by borrowing an entity retained by their owner:
///
/// ```
/// # use rake::gpui::{Entity, IntoElement, Render};
/// fn mounted<V: Render>(editor: &Entity<V>) -> impl IntoElement {
///     rake::view! { <view entity={&editor}/> }
/// }
/// ```
///
/// Typed actions, scoped keymaps, and opt-in Tab navigation use GPUI directly:
///
/// ```
/// use rake::gpui::{self, FocusHandle, IntoElement};
/// gpui::actions!(editor, [Save]);
/// fn toolbar(focus: &FocusHandle, busy: bool) -> impl IntoElement {
///     rake::view! {
///         <section focus={&focus} key-context="Editor" tab-navigation={true}
///             on:Save={|_: &Save, _, _| {}}>
///             <button disabled={busy} aria-keyshortcuts="Control+S"
///                 on:click={|_, window, cx| window.dispatch_action(Box::new(Save), cx)}>
///                 "Save"
///             </button>
///         </section>
///     }
/// }
/// ```
///
/// Register key bindings once during application setup. Disabled buttons skip Tab
/// and suppress activation; enclosing action handlers must check availability too.
/// A focused button retains its focus anchor when disabled so Tab can leave it.
///
/// Action attributes require real GPUI action types:
///
/// ```compile_fail
/// struct NotAnAction;
/// let _ = rake::view! { <div on:NotAnAction={|_, _, _| {}}/> };
/// ```
///
/// Uppercase tags construct typed GPUI components. Paired tags supply a children
/// vector; named slots can be ordinary properties such as `header: AnyElement`.
///
/// ```
/// use rake::gpui::{self, prelude::*};
/// #[derive(IntoElement)]
/// struct Stack {
///     id: gpui::ElementId,
///     header: gpui::AnyElement,
///     children: Vec<gpui::AnyElement>,
/// }
/// impl RenderOnce for Stack {
///     fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
///         gpui::div().id(self.id).flex().flex_col()
///             .child(self.header).children(self.children)
///     }
/// }
/// let _stack = rake::view! {
///     <Stack id={"stack".into()} header={<h2>Typed children</h2>}>
///         <p>Ordinary GPUI elements.</p>
///     </Stack>
/// };
/// ```
///
/// Component properties follow Rust field types, visibility, and ownership rules.
/// Markup-valued properties such as `header={<h2>Title</h2>}` produce `AnyElement`.
/// Expression properties retain their types without implicit conversions. Component
/// root IDs must be forwarded to the rendered GPUI root. Retain state in caller-owned entities.
///
/// ```compile_fail
/// struct Badge { label: String }
/// let _ = rake::view! { <Badge label={42}/> };
/// ```
///
/// ```compile_fail
/// struct Badge { label: String, highlighted: bool }
/// let _ = rake::view! { <Badge label={"Label".into()}/> };
/// ```
///
/// ```compile_fail
/// struct Badge { label: String }
/// let _ = rake::view! { <Badge label={"Label".into()} unknown={true}/> };
/// ```
///
/// ```compile_fail
/// struct Badge { label: String }
/// let _ = rake::view! { <Badge label={"Label".into()}>"No children field"</Badge> };
/// ```
///
/// Paragraphs compose string-like expressions and inline styles into one text layout:
///
/// ```
/// use rake::gpui;
/// let labels = ["Ada", "Lin"];
/// let _paragraph = rake::view! {
///     <p wrap={true} max-lines={2} ellipsis={true}
///         style={|s| s.w(gpui::px(240.))}>
///         <strong>"Hello, "</strong>
///         for (index, label) in labels.iter().enumerate() {
///             if index > 0 { ", " }
///             <span class="italic underline">{label}</span>
///         }
///         <br/>"One shaped paragraph."
///     </p>
/// };
/// ```
///
/// Inline `style` accepts `gpui::HighlightStyle`; sizes and layout belong on `p`.
/// Paragraphs preserve the complete accessible text when visually truncated.
/// `max-lines={0}` normalizes to one. Selection and inline links are not implemented.
/// Use `div`/`section` for layout and embedded widgets; paragraph expressions must
/// implement `AsRef<str>`:
///
/// ```compile_fail
/// let _ = rake::view! { <p>{rake::gpui::div()}</p> };
/// ```
///
/// ```compile_fail
/// let _ = rake::view! { <p><button>"A widget"</button></p> };
/// ```
pub use rake_macros::{Theme, style, view};
pub use router::Route;
pub use schemars;
pub use serde;
pub use serde_json;
pub use services::{ServiceError, ServiceRegistry, Services};
pub use tracing::{debug, error, info, trace, warn};

/// Common application and service imports.
pub mod prelude {
    pub use crate::control::{
        ApplicationCommand, CommandContext, CommandDescriptor, CommandError, Commands,
        ControlConfig, ExecutionCapability, RuntimeClient,
    };

    pub use crate::diagnostics::{Collector, DiagnosticsConfig, SubscriberMode};
    pub use crate::http::{CancellationToken, HttpClient, HttpConfig};
    pub use crate::quit::{QuitJobs, QuitOptions, QuitPhase, QuitResult, QuitStatus};
    pub use crate::reload::{Checkpoint, ReloadError, ReloadState};
    pub use crate::services::{ServiceError, ServiceRegistry, Services};
    pub use crate::tasks::{ActivityRun, AppHandle, TaskError, TaskHandle, TaskScope};

    pub use crate::router::{
        Activation, DecisionChoice, DecisionRequest, EntryHandle, GuardResult, Navigator,
        OpenOptions, OpenTarget, Outcome, RevealScope, Route, Router, RouterConfig, TabMode,
        WebConfig, WebRouting,
    };

    #[cfg(feature = "gpui")]
    pub use crate::App;
    #[cfg(feature = "gpui")]
    pub use crate::inspection::InspectElement;
    #[cfg(feature = "gpui")]
    pub use crate::router::{PageContext, RouteOutlet, RouteViews, TabBar};
    pub use rake_macros::{Command, Route};
}

#[doc(hidden)]
pub mod __private {
    #[cfg(feature = "experimental-view")]
    pub use crate::theme::apply::{
        ThemeBackground, ThemeBorder, ThemeFont, ThemeRadius, ThemeRunFont, ThemeRunText, ThemeText,
    };

    #[cfg(feature = "experimental-view")]
    pub use crate::view::{
        Children as ViewChildren, ComponentChildren as ViewComponentChildren,
        ParagraphContent as ViewParagraphContent, RunStyle as ViewRunStyle, button as view_button,
        entity as view_entity, focus as view_focus, image as view_image, svg as view_svg,
        tab_index as view_tab_index, tab_navigation as view_tab_navigation,
    };

    /// Deprecated macro event aliases; referenced at the authored attribute span.
    #[cfg(feature = "experimental-view")]
    pub mod view_legacy {
        #[deprecated(note = "use on:click")]
        pub const ONCLICK: () = ();

        #[deprecated(note = "use on:key-down")]
        pub const ONKEYDOWN: () = ();

        #[deprecated(note = "use on:key-up")]
        pub const ONKEYUP: () = ();

        #[deprecated(note = "use on:mouse-move")]
        pub const ONMOUSEMOVE: () = ();

        #[deprecated(note = "use on:mouse-down")]
        pub const ONMOUSEDOWN: () = ();

        #[deprecated(note = "use on:mouse-up")]
        pub const ONMOUSEUP: () = ();
    }

    pub use tracing;
    #[cfg(target_family = "wasm")]
    pub use wasm_bindgen;
}

/// Declares this crate's ambient view theme; invoke once at the crate root.
/// Install the value with [`theme::install`] before building themed views.
#[cfg(feature = "experimental-view")]
#[macro_export]
macro_rules! theme {
    ($theme:path) => {
        #[doc(hidden)]
        pub(crate) type __RakeTheme = $theme;
    };
}
