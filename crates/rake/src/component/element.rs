use std::any::Any;
use std::marker::PhantomData;

use crate::gpui::{
    AnyElement, App, AppContext, Bounds, Element, ElementId, Entity, GlobalElementId,
    InspectorElementId, IntoElement, LayoutId, Pixels, StyleRefinement, Window,
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
    marker: PhantomData<fn() -> P>,
}

#[doc(hidden)]
pub struct ComponentLayout {
    owner: Entity<Instance>,
    child: AnyElement,
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
    type RequestLayoutState = ComponentLayout;
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
                    let changed = mounted.entity.update(cx, |instance, _| {
                        let previous = (instance.props.as_mut() as &mut dyn Any)
                            .downcast_mut::<P>()
                            .unwrap();
                        let changed = !props.equivalent(previous);
                        *previous = props;
                        changed
                    });
                    ((mounted.entity.clone(), changed), mounted)
                } else {
                    let registry = cx.default_global::<Registry>().clone();
                    let entity = cx.new(|cx| Instance {
                        props: Box::new(props),
                        hooks: Hooks::default(),
                        registration: registry.register(cx.weak_entity()),
                    });
                    (
                        (entity.clone(), false),
                        Mounted {
                            entity,
                            marker: PhantomData,
                        },
                    )
                }
            });
        let mut child = match self.cache_style.take().filter(|_| !changed) {
            Some(style) => entity.clone().cached(style).into_any_element(),
            None => entity.clone().into_any_element(),
        };
        let layout = child.request_layout(window, cx);

        (
            layout,
            ComponentLayout {
                owner: entity,
                child,
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        #[cfg(feature = "devtools")]
        {
            let hooks = &state.owner.read(cx).hooks;
            let (metadata, truncated) = super::inspection::capture(hooks);
            crate::inspection::component_hooks(
                cx,
                std::any::type_name::<Self>(),
                metadata,
                truncated,
            );
        }

        state.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        ComponentLayout { owner, child }: &mut Self::RequestLayoutState,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        child.paint(window, cx);
        let hooks = &owner.read(cx).hooks;

        if hooks.pending_effects && !hooks.effect_queued {
            owner.update(cx, |instance, _| instance.hooks.effect_queued = true);
            let owner = owner.downgrade();
            cx.defer(move |cx| flush_effects(owner, cx));
        }
    }
}
