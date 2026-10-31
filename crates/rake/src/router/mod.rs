//! Typed navigation, retained independent histories and revision-checked operations.
//!
//! `Router` owns local state and resources. `Navigator` is a weak command proxy:
//! submission queues eagerly, while one host-owned driver executes local futures.
//!
//! ```
//! use rake::prelude::*;
//!
//! #[derive(Clone, Route)]
//! enum Destination {
//!     #[route(id = "home", path = "/")]
//!     Home,
//!     #[route(id = "document", path = "/documents/{id}")]
//!     Document(u64),
//! }
//!
//! assert_eq!(Destination::Document(42).to_uri().unwrap(), "/documents/42");
//! let router = Router::<Destination>::new(RouterConfig::default());
//! let operation = router.navigator().open(Destination::Home, OpenTarget::NewTab);
//! assert_eq!(router.snapshot().operations.len(), 1); // Eagerly queued.
//! // A GPUI host drives router.run(); polling operation.wait() is observation only.
//! ```
//!
//! Ambiguous schemas fail at compile time:
//!
//! ```compile_fail
//! #[derive(Clone, rake::Route)]
//! enum Bad {
//!     #[route(id = "same", path = "/a")]
//!     First,
//!     #[route(id = "same", path = "/b")]
//!     Second,
//! }
//! ```

pub(crate) mod runtime;
mod types;
#[cfg(feature = "gpui")]
pub(crate) mod view;
pub(crate) mod web;

pub use rake_protocol::router::*;
pub use runtime::{EntryHandle, Navigator, Router, TabHandle};
pub use types::*;
#[cfg(feature = "gpui")]
pub use view::{PageContext, RouteOutlet, RouteViews, TabBar};
pub use web::{BrowserCheckpoint, HistoryChange, WebConfig, WebRouting};
