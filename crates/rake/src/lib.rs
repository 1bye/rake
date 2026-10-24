//! Rake's portable application foundation.
//!
//! Default builds include the pinned GPUI native/browser adapters. Disable default
//! features for HTTP, data and diagnostics consumers that do not need a UI.

extern crate self as rake;

pub mod control;
pub mod diagnostics;
pub mod http;
pub mod inspection;
pub mod reload;
pub mod router;
pub mod tasks;

#[cfg(feature = "gpui")]
mod application;

#[cfg(feature = "gpui")]
pub use application::{App, AppError, Services};
#[cfg(feature = "gpui")]
pub use gpui;
pub use rake_macros::{Command, Route, app, data, instrument};
pub use router::Route;
pub use schemars;
pub use serde;
pub use serde_json;
pub use tracing::{debug, error, info, trace, warn};

/// Common application and service imports.
pub mod prelude {
    pub use crate::control::{
        ApplicationCommand, CommandContext, CommandDescriptor, CommandError, Commands,
        ControlConfig, ExecutionCapability, RuntimeClient,
    };

    pub use crate::diagnostics::{Collector, DiagnosticsConfig, SubscriberMode};
    pub use crate::http::{CancellationToken, HttpClient, HttpConfig};
    pub use crate::reload::{Checkpoint, ReloadError, ReloadState};
    pub use crate::tasks::{ActivityRun, AppHandle, TaskError, TaskHandle, TaskScope};

    pub use crate::router::{
        Activation, DecisionChoice, DecisionRequest, EntryHandle, GuardResult, Navigator,
        OpenOptions, OpenTarget, Outcome, RevealScope, Route, Router, RouterConfig, TabMode,
        WebConfig, WebRouting,
    };

    #[cfg(feature = "gpui")]
    pub use crate::inspection::InspectElement;
    #[cfg(feature = "gpui")]
    pub use crate::router::{PageContext, RouteOutlet, RouteViews, TabBar};
    #[cfg(feature = "gpui")]
    pub use crate::{App, Services};
    pub use rake_macros::{Command, Route};
}

#[doc(hidden)]
pub mod __private {
    pub use tracing;
    #[cfg(target_family = "wasm")]
    pub use wasm_bindgen;
}
