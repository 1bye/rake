//! Immutable shared details stay off the per-frame tree and are copied only for wire replies.

use super::{Element, MAX_CAPTURE_BYTES, ResolvedStyle, Snapshot, StyleOrigin, budget};
use serde::Serialize;
use serde_json::Value;

use std::{
    ops::{Deref, DerefMut},
    rc::Rc,
};

pub(super) struct Payload<T> {
    pub value: T,
    pub bytes: usize,
}

impl<T: Serialize> Payload<T> {
    pub fn new(value: T, limit: usize) -> Rc<Self> {
        let bytes = budget::size(&value, limit);

        Rc::new(Self { value, bytes })
    }
}

#[derive(Clone, Default)]
pub(super) struct Details {
    pub styles: Option<Rc<Payload<Value>>>,
    pub resolved: Option<Rc<Payload<ResolvedStyle>>>,
    pub hovered: bool,
    pub measured: bool,
}

impl Details {
    pub fn apply(&self, element: &mut Element) {
        element.styles = self.styles.as_ref().map(|style| style.value.clone());
        element.resolved = self.resolved.as_ref().map(|style| style.value.clone());

        if self.measured
            && let Some(resolved) = &mut element.resolved
        {
            resolved.hovered = self.hovered;

            if let Some(layout) = element.layout {
                let size = serde_json::json!({"width":format!("{}px", layout.width), "height":format!("{}px", layout.height)});

                resolved.properties["size"] = size.clone();

                for layer in &mut resolved.layers {
                    if layer.origin == StyleOrigin::Layout {
                        layer.properties["size"] = size.clone();
                    }
                }
            } else {
                resolved
                    .properties
                    .as_object_mut()
                    .map(|properties| properties.remove("size"));

                for layer in &mut resolved.layers {
                    if layer.origin == StyleOrigin::Layout {
                        layer
                            .properties
                            .as_object_mut()
                            .map(|properties| properties.remove("size"));
                    }
                }
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.styles.as_ref().map_or(0, |style| style.bytes)
            + self.resolved.as_ref().map_or(0, |style| {
                style.bytes + 16 + if self.measured { 256 } else { 0 }
            })
    }
}

#[derive(Clone)]
pub(super) struct Captured {
    pub tree: Snapshot,
    pub index_revision: String,
    pub details: Vec<Details>,
}

impl From<Snapshot> for Captured {
    fn from(mut tree: Snapshot) -> Self {
        let details = tree
            .elements
            .iter_mut()
            .map(|element| Details {
                styles: element
                    .styles
                    .take()
                    .map(|value| Payload::new(value, MAX_CAPTURE_BYTES)),
                hovered: false,
                measured: false,
                resolved: element
                    .resolved
                    .take()
                    .map(|value| Payload::new(value, MAX_CAPTURE_BYTES)),
            })
            .collect();

        Self {
            index_revision: tree.capture.clone(),
            tree,
            details,
        }
    }
}

impl Captured {
    pub fn element(&self, index: usize) -> Element {
        let mut element = self.tree.elements[index].clone();

        if let Some(details) = self.details.get(index) {
            details.apply(&mut element);
        }

        element
    }

    #[cfg(test)]
    pub fn owned(&self) -> Snapshot {
        let mut snapshot = self.tree.clone();

        for (element, details) in snapshot.elements.iter_mut().zip(&self.details) {
            details.apply(element);
        }

        snapshot
    }
}

impl Deref for Captured {
    type Target = Snapshot;

    fn deref(&self) -> &Snapshot {
        &self.tree
    }
}

impl DerefMut for Captured {
    fn deref_mut(&mut self) -> &mut Snapshot {
        &mut self.tree
    }
}
