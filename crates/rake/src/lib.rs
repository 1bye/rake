//! Rake's portable application foundation.
//!
//! Default builds include the pinned GPUI native/browser adapters. Disable default
//! features for HTTP, data and diagnostics consumers that do not need a UI.
//! Inspection and temporary style editing require the opt-in `devtools` feature;
//! ordinary native and WASM builds omit their instrumentation and retained state.

extern crate self as rake;

/// Optional, GPUI-independent per-user launch-at-login settings.
#[cfg(feature = "launch-at-login")]
pub use rake_autostart as login_item;

#[path = "inspection/macros.rs"]
mod inspection_macros;

pub use rake_resources as resources;
pub use rake_resources::{Resources, resources};

/// Optional native components; currently exposes the macOS AppKit adapter.
#[cfg(feature = "native-components")]
pub use rake_native as native;

#[cfg(feature = "updates")]
pub use rake_updates as updates;
#[cfg(feature = "updates")]
pub use rake_updates::updates;

/// Runs enabled native updater installer callbacks before constructing the app.
/// Manual entry points use `early_startup!()`; `#[rake::app]` does so automatically.
/// # Panics
/// Panics if a hand-written embedded policy is invalid; rake-build validates generated policy.
pub fn early_startup_for(embedded: Option<&str>) {
    early_startup_with_desktop(embedded, None);
}

/// Runs installer startup with common desktop metadata captured by [`early_startup!`].
/// # Panics
/// Panics if hand-written embedded metadata is invalid; rake-build validates generated metadata.
pub fn early_startup_with_desktop(embedded: Option<&str>, desktop: Option<&str>) {
    #[cfg(all(feature = "updates", not(target_family = "wasm")))]
    rake_updates::early_startup_with_desktop(embedded, desktop)
        .expect("generated Rake distribution policy must be valid");
    #[cfg(any(not(feature = "updates"), target_family = "wasm"))]
    let _ = (embedded, desktop);
}

/// Initializes installer callbacks using policy embedded into the application crate.
#[macro_export]
macro_rules! early_startup {
    () => {
        $crate::early_startup_with_desktop(
            option_env!("RAKE_UPDATE_POLICY"),
            option_env!("RAKE_DESKTOP_METADATA"),
        )
    };
}

pub mod control;
pub use rake_macros::Props;

#[doc = "Experimental signals backed by typed GPUI entities."]
#[cfg(feature = "experimental-reactive")]
pub mod reactive;
#[cfg(feature = "experimental-reactive")]
pub use rake_macros::component;

pub mod crash;

#[cfg(all(feature = "devtools-memory", unix))]
pub mod memory;

pub use rake_diagnostics as diagnostics;

pub use rake_http as http;

#[cfg(feature = "devtools")]
pub mod inspection;
#[cfg(not(feature = "devtools"))]
#[path = "inspection/disabled.rs"]
pub mod inspection;
pub mod loading;
#[cfg(feature = "gpui")]
pub mod menus;
pub mod platform;
pub mod quit;
pub mod reload;
pub mod router;
pub mod runtime;
pub mod services;
pub mod storage;
#[cfg(all(
    feature = "gpu-surface",
    any(target_os = "macos", target_os = "linux", target_os = "windows")
))]
pub mod surface;
pub mod tasks;
#[cfg(feature = "tray")]
pub mod tray;
pub mod web;
pub mod window_state;

#[cfg(all(feature = "gpui", feature = "updates"))]
mod updates_binding;

#[cfg(all(feature = "gpui", feature = "updates"))]
pub use updates_binding::{UpdateHandle, UpdateHandleState};

#[cfg(feature = "gpui")]
mod application;

#[cfg(feature = "gpui")]
use rake_ui::view;

#[cfg(feature = "gpui")]
pub mod overlay;

#[cfg(feature = "gpui")]
pub use rake_ui::theme;

/// Text input components for GPUI views.
#[cfg(feature = "gpui")]
pub mod ui {
    pub use rake_ui::input;
}

/// Initializes Base, optional Kit and theme observation before custom GPUI windows.
/// Rake's application builder calls this automatically.
#[cfg(feature = "gpui")]
pub use rake_ui::init as init_ui;

/// Optional GPUI Kit components, assets and the matching GPUI API (native targets).
#[cfg(all(feature = "kit", not(target_family = "wasm")))]
pub use rake_ui::kit;

#[cfg(feature = "gpui")]
pub use application::{App, AppError, ClosePolicy, show_main_window};
#[cfg(feature = "gpui-fast")]
pub extern crate gpui_fast as gpui;
#[cfg(feature = "gpui-fast")]
extern crate gpui_fast_platform as gpui_platform;
#[cfg(all(feature = "gpui", not(feature = "gpui-fast")))]
pub extern crate gpui_pre as gpui;
pub use rake_macros::{Command, Route, app, data, instrument};
#[cfg(feature = "gpui")]
#[doc = include_str!("../view-macro.md")]
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
    #[cfg(feature = "experimental-reactive")]
    pub use crate::reactive::{
        Children, Memo, ReactiveApp, Resource, Setup, Signal, SignalContext,
    };

    #[cfg(all(feature = "gpui", feature = "updates"))]
    pub use crate::{UpdateHandle, UpdateHandleState};

    pub use crate::diagnostics::{Collector, DiagnosticsConfig, SubscriberMode};
    pub use crate::http::{CancellationToken, HttpClient, HttpConfig};
    #[cfg(feature = "gpui")]
    pub use crate::loading::{AsyncAction, LoadContext, LoadError, Loader};
    pub use crate::loading::{LoadState, LoadStatus, LoadToken};

    pub use crate::quit::{
        QuitGuardContext, QuitJobs, QuitOptions, QuitPhase, QuitResult, QuitStatus,
    };

    pub use crate::reload::{Checkpoint, ReloadError, ReloadState};
    pub use crate::runtime::{Runtime, RuntimeContext};
    pub use crate::services::{ServiceError, ServiceRegistry, Services};
    pub use crate::tasks::{ActivityRun, AppHandle, TaskError, TaskHandle, TaskScope};

    pub use crate::router::{
        Activation, Admission, AdmissionContext, DecisionChoice, DecisionRequest, EntryHandle,
        GuardResult, Navigator, OpenOptions, OpenTarget, Outcome, RevealScope, Route, Router,
        RouterConfig, TabMode, WebConfig, WebRouting,
    };

    #[cfg(feature = "gpui")]
    pub use crate::App;
    #[cfg(feature = "gpui")]
    pub use crate::inspection::InspectElement;

    #[cfg(feature = "gpui")]
    pub use crate::router::{
        LayoutContext, LoadedPage, NavigationBoundary, Page, PageContext, RouteOutlet, RouteViews,
        TabBar,
    };

    pub use rake_macros::{Command, Route};
}

#[doc(hidden)]
pub mod __private {
    pub use crate::__rake_inspection as inspection;
    #[cfg(feature = "gpui")]
    pub use crate::application::{AppFactoryOutput, run_app_factory};
    #[cfg(feature = "gpui")]
    pub use rake_ui::__private::{
        EquivalentProp, PropComparison, PropMissing, PropSet, PropsBuilder, RequiredProp, Value,
    };

    #[cfg(all(feature = "gpui", feature = "devtools"))]
    pub use crate::inspection::authored::{
        Definition as ViewDefinition, HandlerSite as ViewHandlerSite, StyleSite as ViewStyleSite,
        child as view_inspect_child, component as view_inspect_component,
        component_key as view_component_key, styled as view_inspect,
    };

    #[cfg(feature = "gpui")]
    pub use crate::theme::apply::{
        ThemeBackground, ThemeBorder, ThemeFont, ThemeRadius, ThemeRunFont, ThemeRunText, ThemeText,
    };

    #[cfg(feature = "gpui")]
    pub use crate::view::{
        Children as ViewChildren, ComponentChildren as ViewComponentChildren,
        ParagraphContent as ViewParagraphContent, RunStyle as ViewRunStyle, button as view_button,
        button_base as view_button_base, entity as view_entity, focus as view_focus,
        image as view_image, svg as view_svg, tab_index as view_tab_index,
        tab_navigation as view_tab_navigation,
    };

    /// Deprecated macro event aliases; referenced at the authored attribute span.
    #[cfg(feature = "gpui")]
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
#[cfg(feature = "gpui")]
#[macro_export]
macro_rules! theme {
    ($theme:path) => {
        #[doc(hidden)]
        pub(crate) type __RakeTheme = $theme;
    };
}
