use std::fmt::Display;

use crate::gpui::{App, IntoElement, RenderOnce, ViewElement, Window};

use super::{Setup, Signal, tracking};

#[doc = "A copyable, read-only derived signal. Dependencies follow the reads made by its computation."]
pub struct Memo<T: 'static>(pub(super) Signal<T>);

impl<T> Copy for Memo<T> {}
impl<T> Clone for Memo<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Clone + 'static> super::Value<T> for Memo<T> {
    fn resolve(self, cx: &App) -> T {
        self.get(cx)
    }
}

impl<T: 'static> Memo<T> {
    #[doc(hidden)]
    pub fn from_signal(signal: Signal<T>) -> Self {
        Self(signal)
    }

    #[doc = "Borrows the current derived value and tracks its readers."]
    pub fn read(self, cx: &App) -> &T {
        self.0.read(cx)
    }

    #[doc = "Clones the current derived value."]
    pub fn get(self, cx: &App) -> T
    where
        T: Clone,
    {
        self.read(cx).clone()
    }

    #[doc = "Reads the derived value without cloning it."]
    pub fn with<R>(self, cx: &App, read: impl FnOnce(&T) -> R) -> R {
        read(self.read(cx))
    }
}

impl<V: 'static> Setup<'_, '_, V> {
    #[doc = "Computes an equality-checked value and observes the signals read by the computation."]
    pub fn memo<T: PartialEq + 'static>(
        &mut self,
        compute: impl Fn(&App) -> T + 'static,
    ) -> Memo<T> {
        let (value, dependencies) = tracking::collect(self, |cx| compute(cx));
        let output = self.signal(value);
        let observer = tracking::Observer::new(
            self,
            move |cx| output.set_if_changed(cx, compute(cx)),
            dependencies,
        );
        self.retain(observer);
        Memo(output)
    }
}

impl<T: Display + 'static> IntoElement for Memo<T> {
    type Element = ViewElement<Self>;
    fn into_element(self) -> Self::Element {
        ViewElement::new(self)
    }
}
impl<T: Display + 'static> RenderOnce for Memo<T> {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.read(cx).to_string()
    }
}
impl<T: Display + 'static> IntoElement for Signal<T> {
    type Element = ViewElement<Self>;
    fn into_element(self) -> Self::Element {
        ViewElement::new(self)
    }
}
impl<T: Display + 'static> RenderOnce for Signal<T> {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        self.read(cx).to_string()
    }
}
