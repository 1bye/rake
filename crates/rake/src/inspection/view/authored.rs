//! Macro-authored boundaries. Static source descriptors never evaluate user expressions.

use std::{
    any::Any,
    ops::{Deref, DerefMut},
    panic::Location,
};

use gpui::{Element, ElementId, IntoElement, ParentElement, Styled};

use super::{Inspected, inspect, limited};
use crate::inspection::{SourceLocation, StyleSource, ViewMetadata};

#[doc(hidden)]
pub struct StyleSite {
    pub scope: &'static str,
    pub attribute: &'static str,
    pub expression: &'static str,
    pub source: &'static Location<'static>,
}

#[doc(hidden)]
pub struct HandlerSite {
    pub name: &'static str,
    pub source: &'static Location<'static>,
}

#[doc(hidden)]
pub struct Definition {
    pub tag: &'static str,
    pub site: &'static str,
    pub source: &'static Location<'static>,
    pub styles: &'static [StyleSite],
    pub handlers: &'static [HandlerSite],
}

impl Definition {
    pub(super) fn capture(&self, kind: &str, truncated: &mut bool) -> ViewMetadata {
        let tag = if self.tag == "Expression" {
            crate::inspection::display_name(kind)
        } else {
            self.tag
        };
        let mut metadata = ViewMetadata {
            tag: limited(tag, 256, truncated),
            style_sources: Vec::new(),
        };
        const MAX_SOURCE_BYTES: usize = 4096;
        let mut bytes = super::super::budget::size(&metadata, MAX_SOURCE_BYTES);

        for site in self.styles {
            let source = StyleSource {
                scope: limited(site.scope, 128, truncated),
                attribute: limited(site.attribute, 64, truncated),
                expression: limited(site.expression, 512, truncated),
                source: SourceLocation {
                    file: limited(site.source.file(), 512, truncated),
                    line: site.source.line(),
                    column: site.source.column(),
                },
            };
            bytes += super::super::budget::size(&source, MAX_SOURCE_BYTES) + 1;

            if bytes > MAX_SOURCE_BYTES {
                *truncated = true;
                break;
            }
            metadata.style_sources.push(source);
        }
        metadata
    }
}

#[doc(hidden)]
pub fn styled<E: Element + Styled + IntoElement<Element = E>>(
    inner: E,
    definition: &'static Definition,
    named: bool,
) -> Inspected<E> {
    let semantic = named.then(|| inner.id().map(|id| id.to_string())).flatten();
    let mut element = inspect(inner);
    element.semantic_id = semantic;
    element.source = definition.source;
    element.definition = Some(definition);
    element.style_reader = Some(|inner| inner.style().clone());
    element
}

/// Delays component conversion so typed properties remain available before rendering.
#[doc(hidden)]
pub struct Component<C> {
    inner: C,
    definition: &'static Definition,
    key: fn(&C) -> Option<ElementId>,
}

impl<C> Component<C> {
    /// Extracts the original component when direct ownership of its properties is needed.
    pub fn into_inner(self) -> C {
        self.inner
    }
}

impl<C> Deref for Component<C> {
    type Target = C;
    fn deref(&self) -> &C {
        &self.inner
    }
}

impl<C> DerefMut for Component<C> {
    fn deref_mut(&mut self) -> &mut C {
        &mut self.inner
    }
}

impl<C: IntoElement> IntoElement for Component<C> {
    type Element = Inspected<C::Element>;

    fn into_element(self) -> Self::Element {
        let key = (self.key)(&self.inner);
        let mut element = inspect(self.inner);
        element.semantic_id = key.as_ref().map(ToString::to_string);
        element.view_key = key;
        element.source = self.definition.source;
        element.definition = Some(self.definition);
        element
    }
}

#[doc(hidden)]
pub fn component<C>(
    inner: C,
    definition: &'static Definition,
    key: fn(&C) -> Option<ElementId>,
) -> Component<C> {
    Component {
        inner,
        definition,
        key,
    }
}

/// Optional inspection identity; arbitrary typed component properties stay unconstrained.
#[doc(hidden)]
pub fn component_key(value: &dyn Any) -> Option<ElementId> {
    macro_rules! key {
        ($($ty:ty),* $(,)?) => { $(if let Some(value) = value.downcast_ref::<$ty>() { return Some(value.clone().into()); })* };
    }
    key!(
        ElementId,
        String,
        &'static str,
        gpui::SharedString,
        usize,
        i32,
        (&'static str, usize),
        (&'static str, u64),
        (&'static str, u32),
        (gpui::SharedString, usize),
        uuid::Uuid
    );
    if let Some(value) = value.downcast_ref::<u64>() {
        return Some(ElementId::Integer(*value));
    }
    if let Some(value) = value.downcast_ref::<u32>() {
        return Some(ElementId::Integer(u64::from(*value)));
    }
    None
}

#[doc(hidden)]
#[track_caller]
pub fn child<E: IntoElement>(
    siblings: &mut rake_ui::view::Children,
    parent: &mut impl ParentElement,
    value: E,
    id: ElementId,
    definition: &'static Definition,
) {
    let element = value.into_element();
    let any = &element as &dyn Any;
    if let Some(text) = any.downcast_ref::<gpui::SharedString>() {
        push(
            siblings,
            parent,
            gpui::Text::new(id.clone(), text.clone()),
            id,
            definition,
        );
    } else if let Some(text) = any.downcast_ref::<&'static str>() {
        push(
            siblings,
            parent,
            gpui::Text::new(id.clone(), gpui::SharedString::new_static(text)),
            id,
            definition,
        );
    } else {
        push(siblings, parent, element, id, definition);
    }
}

#[track_caller]
fn push<E: Element>(
    siblings: &mut rake_ui::view::Children,
    parent: &mut impl ParentElement,
    inner: E,
    key: ElementId,
    definition: &'static Definition,
) {
    let mut element = inspect(inner);
    element.source = definition.source;
    element.definition = Some(definition);
    element.view_key = Some(key);
    siblings.push(parent, element);
}

#[cfg(all(test, feature = "view-test-support"))]
mod tests;
