use std::rc::Rc;

use crate::gpui::{App, Window};

type Callback<T> = dyn Fn(&T, &mut Window, &mut App);

#[doc = "A cloneable callback property. The event and contexts stay borrowed for the call."]
pub struct Event<T>(Rc<Callback<T>>);

impl<T> Clone for Event<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Default for Event<T> {
    fn default() -> Self {
        Self(Rc::new(|_, _, _| {}))
    }
}

impl<T> Event<T> {
    #[doc = "Creates an event callback."]
    pub fn new(callback: impl Fn(&T, &mut Window, &mut App) + 'static) -> Self {
        Self(Rc::new(callback))
    }

    #[doc = "Invokes this callback with the current event and GPUI contexts."]
    pub fn call(&self, event: &T, window: &mut Window, cx: &mut App) {
        (self.0)(event, window, cx);
    }
}
