use std::rc::Rc;

use crate::gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

use super::component::{ReactiveView, StretchComponent};
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

// This wrapper cannot cache. Only its mounted entity needs GPUI's view style storage.
#[doc(hidden)]
pub struct Stretched<T> {
    props: Option<T>,
    stretched: bool,
    #[cfg(all(debug_assertions, not(feature = "live")))]
    source: &'static std::panic::Location<'static>,
}

impl<T> Stretched<T> {
    #[doc(hidden)]
    #[track_caller]
    pub fn new(props: T, stretched: bool) -> Self {
        Self {
            props: Some(props),
            stretched,
            #[cfg(all(debug_assertions, not(feature = "live")))]
            source: std::panic::Location::caller(),
        }
    }
}

impl<T: StretchComponent> IntoElement for Stretched<T> {
    // GPUI supplies the hot-patchable render entry point for live builds.
    #[cfg(feature = "live")]
    type Element = crate::gpui::ViewElement<Self>;
    #[cfg(not(feature = "live"))]
    type Element = Self;

    #[track_caller]
    fn into_element(self) -> Self::Element {
        #[cfg(feature = "live")]
        return crate::gpui::ViewElement::new(self);

        #[cfg(not(feature = "live"))]
        self
    }
}

#[cfg(feature = "live")]
impl<T: StretchComponent> crate::gpui::RenderOnce for Stretched<T> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.props
            .unwrap()
            .render_stretched(window, cx, self.stretched)
    }
}

#[cfg(not(feature = "live"))]
impl<T: StretchComponent> Element for Stretched<T> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        #[cfg(debug_assertions)]
        return Some(self.source);

        #[cfg(not(debug_assertions))]
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        window.with_id(std::any::type_name::<Self>(), |window| {
            let mut child = self
                .props
                .take()
                .unwrap()
                .render_stretched(window, cx, self.stretched);

            let layout = child.request_layout(window, cx);

            (layout, child)
        })
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_id(std::any::type_name::<Self>(), |window| {
            child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        child: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_id(std::any::type_name::<Self>(), |window| {
            child.paint(window, cx);
        });
    }
}
