use std::cell::RefCell;
use std::rc::Rc;

use crate::gpui::{
    AnyElement, App, AppContext, Bounds, Element, ElementId, Entity, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, StyleRefinement, WeakEntity, Window,
};

use super::{FunctionComponent, Hooks, Instance, Registry, flush_effects};

#[doc = "A keyed mount for a function component. Its GPUI element state owns exactly one component entity."]
pub struct ComponentElement<P: FunctionComponent> {
    props: Option<P>,
    id: ElementId,
    cache_style: Option<StyleRefinement>,
}

struct Mounted<P> {
    entity: Entity<Instance>,
    props: Rc<RefCell<P>>,
}

pub(super) struct ComponentBody {
    pub child: AnyElement,
    pub owner: WeakEntity<Instance>,
}

impl<P: FunctionComponent> ComponentElement<P> {
    #[doc(hidden)]
    pub fn new(props: P) -> Self {
        Self {
            id: props.component_id(),
            props: Some(props),
            cache_style: None,
        }
    }

    #[doc = "Opts into GPUI subtree caching with an explicit layout style. Both dimensions must be definite; memo components skip equal-prop redraws here."]
    pub fn cached(mut self, style: StyleRefinement) -> Self {
        self.cache_style = Some(style);
        self
    }
}

impl<P: FunctionComponent> IntoElement for ComponentElement<P> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<P: FunctionComponent> Element for ComponentElement<P> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let props = self
            .props
            .take()
            .expect("component element is rendered once per frame");
        let (entity, changed) =
            window.with_element_state::<Mounted<P>, _>(id.unwrap(), |mounted, _| {
                if let Some(mounted) = mounted {
                    let changed = !props.equivalent(&mounted.props.borrow());
                    if changed {
                        mounted.entity.update(cx, |_, _| {
                            *mounted.props.borrow_mut() = props;
                        });
                    }
                    ((mounted.entity.clone(), changed), mounted)
                } else {
                    let registry = cx.default_global::<Registry>().clone();
                    let props = Rc::new(RefCell::new(props));
                    let entity = cx.new(|cx| {
                        let hooks = Rc::new(RefCell::new(Hooks::default()));
                        Instance {
                            props: props.clone(),
                            registration: registry.register(cx.weak_entity(), &hooks),
                            hooks,
                        }
                    });
                    ((entity.clone(), false), Mounted { entity, props })
                }
            });
        let mut child = match self.cache_style.take().filter(|_| !changed) {
            Some(style) => entity.cached(style).into_any_element(),
            None => entity.into_any_element(),
        };
        let layout = child.request_layout(window, cx);

        (layout, child)
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
        child.prepaint(window, cx);
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
        child.paint(window, cx);
    }
}

impl IntoElement for ComponentBody {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for ComponentBody {
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
        (self.child.request_layout(window, cx), ())
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
        {
            let owner = self.owner.upgrade().unwrap();
            let instance = owner.read(cx);
            let (metadata, truncated) = super::inspection::capture(&instance.hooks.borrow());
            crate::inspection::component_hooks(cx, instance.props.type_name(), metadata, truncated);
        }

        self.child.prepaint(window, cx);
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
        self.child.paint(window, cx);
        let owner = self.owner.upgrade().unwrap();
        let hooks = owner.read(cx).hooks.clone();
        let mut hooks = hooks.borrow_mut();

        if hooks.pending_effects && !hooks.effect_queued {
            hooks.effect_queued = true;
            let owner = owner.downgrade();
            cx.defer(move |cx| flush_effects(owner, cx));
        }
    }
}
