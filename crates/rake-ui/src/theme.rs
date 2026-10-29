//! App-owned, ambient themes for experimental views.
//!
//! Call [`install`] during application setup and whenever the theme changes.
//! GPUI's UI and view construction run on the same thread, including on Wasm;
//! the ambient copy is thread-local. Install separately for each UI thread.
//! Setting [`ThemeGlobal`] directly is unsupported: it leaves that copy stale.

use std::any::{Any, TypeId, type_name};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, FontWeight, Global, Pixels};

#[doc(hidden)]
pub mod apply;

thread_local! {
    static THEMES: RefCell<HashMap<TypeId, Rc<dyn Any>>> = RefCell::default();
}

/// Observable GPUI global for a theme installed through [`install`].
///
/// Observe it with `cx.observe_global::<ThemeGlobal<MyTheme>>()`. Read the
/// public handle for access inside GPUI callbacks; use [`current`] in views.
pub struct ThemeGlobal<T: 'static>(pub Rc<T>);

impl<T: 'static> Clone for ThemeGlobal<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static> Global for ThemeGlobal<T> {}

/// Replaces this type's theme, notifies global observers and refreshes windows.
/// Existing element styles keep their values until the view is rebuilt.
pub fn install<T: 'static>(cx: &mut App, theme: T) {
    let theme = Rc::new(theme);

    THEMES.with_borrow_mut(|themes| {
        themes.insert(TypeId::of::<T>(), theme.clone());
    });
    cx.set_global(ThemeGlobal(theme));
    cx.refresh_windows();
}

/// Returns the theme on this UI thread, if installed.
pub fn try_current<T: 'static>() -> Option<Rc<T>> {
    THEMES
        .with_borrow(|themes| themes.get(&TypeId::of::<T>()).cloned())
        .and_then(|theme| theme.downcast().ok())
}

/// Returns the current theme on this UI thread.
///
/// # Panics
/// Panics if this theme type has not been installed on the calling thread.
pub fn current<T: 'static>() -> Rc<T> {
    try_current().unwrap_or_else(|| {
        panic!(
            "theme `{}` is not installed; call rake::theme::install(cx, ...) during app setup on this UI thread",
            type_name::<T>()
        )
    })
}

/// A paragraph's font size and optional line height and weight.
/// Text tokens belong on paragraphs or containers, never inline runs.
#[derive(Clone, Copy, Debug)]
pub struct TextToken {
    /// Font size in pixels.
    pub size: Pixels,
    /// Optional line height in pixels.
    pub line_height: Option<Pixels>,
    /// Optional font weight.
    pub weight: Option<FontWeight>,
}
