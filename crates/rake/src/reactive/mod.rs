mod attrs;
mod children;
mod component;
mod context;
mod effect;
mod element;
mod event;
mod global;
mod memo;
mod resource;
mod signal;
mod tracking;

#[cfg(feature = "devtools")]
mod inspection;

pub use attrs::Attrs;
pub use children::Children;
pub use component::{ComponentView, Setup};
pub use context::SignalContext;
pub use effect::ReactiveApp;
pub use event::Event;
pub use global::provide_global;
pub use memo::Memo;
pub use rake_ui::__private::Value;
pub use resource::Resource;
pub use signal::Signal;

#[cfg(all(test, feature = "view-test-support", not(target_family = "wasm")))]
mod tests;

#[cfg(feature = "view-test-support")]
mod stats;
#[cfg(feature = "view-test-support")]
pub use stats::{RuntimeStats, runtime_stats};
