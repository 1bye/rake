use crate::gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, WeakEntity, Window,
};

use super::component::ReactiveView;
use super::tracking::{self, Dependency};

pub(super) struct Rendered<V> {
    pub owner: WeakEntity<V>,
    pub child: AnyElement,
    pub dependencies: Vec<Dependency>,
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
        let (layout, dependencies) =
            tracking::collect(cx, |cx| self.child.request_layout(window, cx));
        self.dependencies.extend(dependencies);
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
        let (_, dependencies) = tracking::collect(cx, |cx| self.child.prepaint(window, cx));
        self.dependencies.extend(dependencies);
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
        let (_, dependencies) = tracking::collect(cx, |cx| self.child.paint(window, cx));
        self.dependencies.extend(dependencies);
        let dependencies = std::mem::take(&mut self.dependencies);
        let _ = self.owner.update(cx, |view, cx| {
            view.update_dependencies(dependencies, cx.entity().into_any().downgrade(), cx);
        });
    }
}
