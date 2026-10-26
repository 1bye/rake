//! Rake's portable application foundation.
//!
//! Default builds include the pinned GPUI native/browser adapters. Disable default
//! features for HTTP, data and diagnostics consumers that do not need a UI.

extern crate self as rake;

pub mod control;
pub mod diagnostics;
pub mod http;
pub mod inspection;
pub mod quit;
pub mod reload;
pub mod router;
pub mod services;
pub mod tasks;

#[cfg(feature = "gpui")]
mod application;

#[cfg(feature = "experimental-view")]
mod view;

#[cfg(feature = "gpui")]
pub use application::{App, AppError};
#[cfg(feature = "gpui")]
pub use gpui;
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
///                 onclick={|_, window, cx| window.dispatch_action(Box::new(Save), cx)}>
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
///     children: Vec<gpui::AnyElement>,
/// }
/// impl RenderOnce for Stack {
///     fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
///         gpui::div().id(self.id).flex().flex_col().children(self.children)
///     }
/// }
/// let _stack = rake::view! {
///     <Stack id={"stack".into()}>
///         <h2>"Typed children"</h2>
///         <p>"Ordinary GPUI elements."</p>
///     </Stack>
/// };
/// ```
///
/// Component properties follow Rust field types, visibility, and ownership rules.
/// No property defaults or implicit conversions are added; component root IDs must
/// be forwarded to the rendered GPUI root. Retain state in caller-owned entities.
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
pub use rake_macros::view;
pub use rake_macros::{Command, Route, app, data, instrument};
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
    pub use crate::view::{
        Children as ViewChildren, ComponentChildren as ViewComponentChildren,
        button as view_button, entity as view_entity, focus as view_focus, image as view_image,
        tab_index as view_tab_index, tab_navigation as view_tab_navigation,
    };

    pub use tracing;
    #[cfg(target_family = "wasm")]
    pub use wasm_bindgen;
}
