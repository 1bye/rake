//! GPUI view construction, ambient themes and owned overlay state.

pub mod overlay;
pub mod theme;
#[doc(hidden)]
pub mod view;

/// Initializes Base, optional Kit and theme observation once per GPUI application.
/// Call before constructing roots or experimental views outside Rake's application builder.
pub fn init(cx: &mut gpui::App) {
    if !cx.has_global::<Initialized>() {
        #[cfg(all(feature = "kit", not(target_family = "wasm")))]
        gpui_kit::init(cx);
        #[cfg(not(all(feature = "kit", not(target_family = "wasm"))))]
        gpui_base::init(cx);
        cx.set_global(Initialized);
        theme::observe(cx);
    }
}

struct Initialized;

impl gpui::Global for Initialized {}

/// Window root shared by Rake views and Base presentation plugins.
pub use gpui_base::Root;

/// The matching optional Kit facade, sharing Base's root and interaction globals.
#[cfg(all(feature = "kit", not(target_family = "wasm")))]
pub use gpui_kit as kit;

#[cfg(all(feature = "kit", target_family = "wasm"))]
compile_error!(
    "Rake's kit feature is native-only: GPUI Kit 0.9.0 enables Web multithreading that requires nightly. Use experimental-view for stable single-threaded Web builds."
);
