//! GPUI view construction, ambient themes and owned overlay state.

#[cfg(feature = "gpui-fast")]
extern crate gpui_fast as gpui;
#[cfg(not(feature = "gpui-fast"))]
extern crate gpui_pre as gpui;

/// GPUI Base text inputs and their retained editing state.
pub mod input {
    pub use gpui_base::input::{
        Input, InputAccessibility, InputEvent, InputState, Textarea, TextareaState,
    };
}

pub mod overlay;
pub mod theme;
#[doc(hidden)]
pub mod view;

/// Initializes Base, optional Kit and theme observation once per GPUI application.
/// Call before constructing roots or views outside Rake's application builder.
/// Adopts an existing Kit initialization without resetting its theme or registry.
pub fn init(cx: &mut gpui::App) {
    if !cx.has_global::<Initialized>() {
        #[cfg(all(feature = "kit", not(target_family = "wasm")))]
        if !cx.has_global::<gpui_kit::component::Theme>() {
            // A Kit host already owns its theme, registry and input bindings.
            gpui_kit::init(cx);
        }

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
    "Rake's kit feature is native-only: GPUI Kit 0.9.0 enables Web multithreading that requires nightly. Use default Rake views for stable single-threaded Web builds."
);

mod props;

#[doc(hidden)]
pub mod __private {
    pub use crate::props::{
        EquivalentProp, PropComparison, PropMissing, PropSet, PropsBuilder, RequiredProp, Value,
    };
}
