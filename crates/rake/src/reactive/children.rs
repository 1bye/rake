use std::rc::Rc;

use crate::gpui::{AnyElement, App, IntoElement, RenderOnce, ViewElement, Window};

type RenderChildren = dyn Fn(&mut Window, &mut App) -> AnyElement;

#[doc = "Re-runnable child content owned by a stateful component."]
#[derive(Clone)]
pub struct Children(Rc<RenderChildren>);

impl Children {
    #[doc = "Stores a factory that builds fresh elements on each render."]
    pub fn new(render: impl Fn(&mut Window, &mut App) -> AnyElement + 'static) -> Self {
        Self(Rc::new(render))
    }
}

impl Default for Children {
    fn default() -> Self {
        Self::new(|_, _| crate::gpui::Empty.into_any_element())
    }
}

impl IntoElement for Children {
    type Element = ViewElement<Self>;

    fn into_element(self) -> Self::Element {
        ViewElement::new(self)
    }
}

impl RenderOnce for Children {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        (self.0)(window, cx)
    }
}
