//! Native tray contracts independent of a windowing toolkit.
//!
//! A host owns native resources; control handles do not extend that host's life.
//! Registration and presentation are distinct: a shell may hide registered items.

mod accelerator;
mod animation;
mod backend;
mod capability;
mod composition;
mod event;
mod host;
mod image;
pub mod linux;
pub mod macos;
mod menu;
mod operations;
mod placement;
mod spec;
pub mod windows;

pub use accelerator::Accelerator;
pub use animation::Animation;
pub use capability::{Backend, Capabilities, Feature, Support};
pub use composition::Corner;

pub use event::{
    Anchor, BalloonEvent, Button, CoordinateSpace, Event, EventKind, Input, Modifiers, Point,
    PointerAction, Rect,
};

pub use host::{Applied, Collection, Handle, Host, Registration, State};
pub use image::{Image, ImageRepresentation, Rendering};
pub use menu::{CheckState, ItemKind, Menu, MenuItem};
pub use placement::PopupAnchor;
pub use spec::{HostPolicy, Interaction, Tray};

/// An initiated local native operation; dropping observation cannot undo committed work.
pub type Request<T> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, Error>>>>;

pub(crate) fn ready<T: 'static>(value: Result<T, Error>) -> Request<T> {
    Box::pin(std::future::ready(value))
}

pub(crate) fn native(
    operation: &'static str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> Error {
    Error::Native {
        operation,
        message: error.to_string(),
        source: Some(std::sync::Arc::new(error)),
    }
}

/// An observable tray failure; native causes remain available through `source()`.
#[derive(Clone, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The request is invalid before touching native resources.
    #[error("invalid tray request: {0}")]
    InvalidInput(String),
    /// An application already owns this stable item name.
    #[error("tray item already exists: {0}")]
    Duplicate(String),
    /// The registration was removed; a reused name is a different registration.
    #[error("tray item has been removed")]
    Removed,
    /// The owning host has stopped or was dropped.
    #[error("tray host has stopped")]
    Stopped,
    /// No compatible native presentation host exists.
    #[error("tray presentation host is unavailable")]
    HostUnavailable,
    /// This backend cannot implement the requested feature.
    #[error("tray feature is unsupported: {0:?}")]
    Unsupported(Feature),
    /// Bounded native operation admission is saturated.
    #[error("tray operation queue is full")]
    QueueFull,
    /// A native operation failed; a diagnostic and optional original cause survive.
    #[error("tray {operation} failed: {message}")]
    Native {
        /// Operation name without user content.
        operation: &'static str,
        /// Native diagnostic or error code.
        message: String,
        /// Original error when supplied by the native API.
        #[source]
        source: Option<std::sync::Arc<dyn std::error::Error + Send + Sync>>,
    },
}

pub(crate) fn text(value: &str, name: &str, max: usize) -> Result<(), Error> {
    if value.contains('\0') || value.len() > max {
        return Err(Error::InvalidInput(format!(
            "{name} contains NUL or exceeds {max} bytes"
        )));
    }

    Ok(())
}

pub(crate) fn identifier(value: &str) -> Result<(), Error> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':'))
    {
        return Err(Error::InvalidInput(
            "IDs require 1–128 ASCII letters, digits, '.', '-', '_' or ':'".into(),
        ));
    }

    Ok(())
}

/// Explicit opt-in fixtures; these are not application integration APIs.
#[cfg(all(feature = "sni-tests", unix))]
#[doc(hidden)]
pub mod native_test {
    /// Exercises the real Linux adapter against an isolated session bus.
    pub async fn sni() -> Result<(), Box<dyn std::error::Error>> {
        crate::backend::sni_probe().await
    }
}
