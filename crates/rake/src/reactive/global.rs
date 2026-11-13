use super::Signal;
use crate::gpui::{App, Global};

pub(super) struct GlobalSignal<T: 'static>(pub Signal<T>);
impl<T: 'static> Global for GlobalSignal<T> {}

#[doc = "Provides application-wide reactive state, updating an existing signal when present."]
pub fn provide_global<T: 'static>(cx: &mut App, value: T) -> Signal<T> {
    if let Some(signal) = cx.try_global::<GlobalSignal<T>>().map(|global| global.0) {
        signal.set(cx, value);
        signal
    } else {
        let signal = Signal::new(cx, value);
        cx.set_global(GlobalSignal(signal));
        signal
    }
}
