//! Framework-free application runtime, route mounts, services and local control.

extern crate self as rake_core;

#[cfg(feature = "launch-at-login")]
pub use rake_autostart as login_item;
#[cfg(feature = "updates")]
pub use rake_updates as updates;
#[cfg(feature = "updates")]
pub use rake_updates::updates;

pub mod control;
pub mod crash;
#[doc(hidden)]
pub mod inspection;
mod inspection_macros;
pub mod loading;
pub mod logging;
#[cfg(all(feature = "devtools-memory", unix))]
pub mod memory;
pub mod platform;
pub mod quit;
pub mod reload;
pub mod router;
pub mod runtime;
pub mod services;
pub mod storage;
pub mod tasks;
pub mod web;
pub mod window_state;

pub use rake_diagnostics as diagnostics;
pub use rake_http as http;
pub use rake_macros::{Command, Props, Route, data, instrument};
pub use rake_resources as resources;
pub use rake_resources::{Resources, resources};
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

    pub use rake_macros::{Command, Route};
}

#[doc(hidden)]
pub mod __private {
    pub use tracing;
    #[cfg(target_family = "wasm")]
    pub use wasm_bindgen;
}
