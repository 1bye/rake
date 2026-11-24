//! Rake application framework. Default builds include the GPUI integration.

#[cfg(not(feature = "gpui"))]
pub use rake_core::*;
#[cfg(feature = "gpui")]
pub use rake_gpui::*;
#[cfg(not(feature = "gpui"))]
pub use rake_macros::app;

#[cfg(all(not(feature = "gpui"), not(feature = "devtools")))]
#[doc(hidden)]
pub use rake_core::__rake_inspection_disabled as __rake_inspection;
#[cfg(all(not(feature = "gpui"), feature = "devtools"))]
#[doc(hidden)]
pub use rake_core::__rake_inspection_enabled as __rake_inspection;

#[cfg(not(feature = "gpui"))]
#[doc(hidden)]
pub mod __private {
    pub use crate::__rake_inspection as inspection;
    pub use rake_core::__private::*;
}

#[cfg(not(feature = "gpui"))]
/// Runs enabled native installer callbacks before constructing the runtime.
pub fn early_startup_for(embedded: Option<&str>) {
    early_startup_with_desktop(embedded, None);
}

#[cfg(not(feature = "gpui"))]
/// Runs installer startup with embedded desktop metadata.
pub fn early_startup_with_desktop(embedded: Option<&str>, desktop: Option<&str>) {
    #[cfg(all(feature = "updates", not(target_family = "wasm")))]
    rake_core::updates::early_startup_with_desktop(embedded, desktop)
        .expect("generated Rake distribution policy must be valid");
    #[cfg(any(not(feature = "updates"), target_family = "wasm"))]
    let _ = (embedded, desktop);
}

#[cfg(not(feature = "gpui"))]
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
