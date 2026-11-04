//! GPUI view construction, ambient themes and owned overlay state.

pub mod overlay;
pub mod theme;
#[doc(hidden)]
pub mod view;

/// Initializes Base's interaction globals and key bindings once per GPUI application.
/// Call before constructing experimental views outside Rake's application builder.
pub fn init(cx: &mut gpui::App) {
    if !cx.has_global::<Initialized>() {
        gpui_base::init(cx);
        cx.set_global(Initialized);
        theme::observe(cx);
    }
}

struct Initialized;

impl gpui::Global for Initialized {}

/// Window root shared by Rake views and Base presentation plugins.
pub use gpui_base::Root;
