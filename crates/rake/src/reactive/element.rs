use std::rc::Rc;

use crate::gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

use super::component::ReactiveView;
use super::tracking::{self, Dependency, RenderSubscriptions};

pub(super) struct Rendered<V> {
    #[cfg(feature = "devtools")]
    pub owner: crate::gpui::WeakEntity<V>,
    #[cfg(not(feature = "devtools"))]
    pub _view: std::marker::PhantomData<V>,
    pub child: AnyElement,
    pub dependencies: Vec<Dependency>,
    pub subscriptions: Rc<RenderSubscriptions>,
}

impl<V: ReactiveView> IntoElement for Rendered<V> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl<V: ReactiveView> Element for Rendered<V> {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let layout = tracking::collect_into(cx, &mut self.dependencies, |cx| {
            self.child.request_layout(window, cx)
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        #[cfg(feature = "devtools")]
        let _ = self.owner.update(cx, |view, cx| view.inspect(cx));
        tracking::collect_into(cx, &mut self.dependencies, |cx| {
            self.child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        tracking::collect_into(cx, &mut self.dependencies, |cx| {
            self.child.paint(window, cx);
        });
        self.subscriptions
            .finish(std::mem::take(&mut self.dependencies), cx);
    }
}
