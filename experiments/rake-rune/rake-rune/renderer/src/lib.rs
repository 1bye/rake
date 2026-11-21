//! Unpublished Rune renderer experiment. No published Rake crate depends on Rune.
pub use rake_rune_macros::view;
pub use rune;

/// Owned child elements; rebuilt from the current state snapshot each frame.
pub type Children = Vec<rune::El>;

/// A component owns its props and consumes its children when rendered.
pub trait Component {
    fn render(self) -> rune::El;
}

/// Values accepted in expression children.
pub trait IntoEl {
    fn into_el(self) -> rune::El;
}
impl IntoEl for rune::El {
    fn into_el(self) -> rune::El {
        self
    }
}
impl IntoEl for String {
    fn into_el(self) -> rune::El {
        rune::text(self)
    }
}
impl IntoEl for &'static str {
    fn into_el(self) -> rune::El {
        rune::text(self)
    }
}

/// A snapshot plus a queued state mutation, not an independently owned store.
pub struct Signal<S> {
    pub checked: bool,
    pub toggle: fn(&mut S),
}
impl<S: 'static> Signal<S> {
    pub fn bind(self, element: rune::El) -> rune::El {
        element.checked(self.checked).on_press(self.toggle)
    }
}
