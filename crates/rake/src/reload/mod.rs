//! Development checkpoint contracts and host coordination.
pub use rake_protocol::reload::*;

#[cfg(any(
    all(feature = "live", not(target_family = "wasm")),
    all(test, not(target_family = "wasm"))
))]
pub(crate) mod live;

#[cfg(any(feature = "gpui", target_family = "wasm"))]
pub(crate) mod host;
