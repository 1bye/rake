//! Window-owned caches keyed by unambiguous presentation identities.

use super::super::{ResolvedStyle, ViewMetadata, budget, snapshot::Payload};
use gpui::{Pixels, StyleRefinement, TextStyle};
use serde_json::Value;
use std::rc::Rc;

#[derive(Default)]
pub(in crate::inspection) struct BoundaryCache {
    budget: Option<(super::super::Element, usize)>,
    authored: Option<(StyleRefinement, Rc<Payload<Value>>)>,
    pub(super) resolved: Option<(StyleInputs, Rc<Payload<ResolvedStyle>>)>,
    #[cfg(feature = "experimental-view")]
    definition: Option<(&'static super::authored::Definition, ViewMetadata, bool)>,
}

#[derive(PartialEq)]
pub(super) struct StyleInputs {
    pub authored: StyleRefinement,
    pub original: Option<StyleRefinement>,
    pub base: StyleRefinement,
    pub live: StyleRefinement,
    pub text: TextStyle,
    pub rem: Pixels,
}

impl BoundaryCache {
    pub(super) fn element_bytes(&mut self, element: &super::super::Element) -> usize {
        debug_assert!(element.styles.is_none() && element.resolved.is_none());

        if let Some((old, bytes)) = &self.budget
            && same_metadata(old, element)
        {
            return *bytes;
        }

        let mut key = element.clone();

        key.id.clear();
        key.parent = None;
        key.stable_id = None;
        key.paint_order = None;
        // IDs, parent IDs, stable identity and paint order have bounded integer spellings.
        let bytes = budget::size(&key, super::super::MAX_CAPTURE_BYTES) + 256;

        self.budget = Some((key, bytes));

        bytes
    }

    pub(super) fn authored(&mut self, style: &StyleRefinement) -> Option<Rc<Payload<Value>>> {
        if let Some((previous, value)) = &self.authored
            && previous == style
        {
            return Some(value.clone());
        }

        let value = serde_json::to_value(style).ok()?;

        if budget::size(&value, 4096) > 4096 {
            self.authored = None;

            return None;
        }

        let value = Payload::new(value, 4096);

        self.authored = Some((style.clone(), Rc::clone(&value)));

        Some(value)
    }

    pub(super) fn resolved(&self, inputs: &StyleInputs) -> Option<Rc<Payload<ResolvedStyle>>> {
        self.resolved
            .as_ref()
            .filter(|(old, _)| old == inputs)
            .map(|(_, style)| style.clone())
    }

    pub(super) fn store_resolved(
        &mut self,
        inputs: StyleInputs,
        style: &Rc<Payload<ResolvedStyle>>,
    ) {
        // Do not retain hidden oversized refinements or styles the capture will reject.
        if budget::size(&inputs.authored, 4096) <= 4096
            && inputs
                .original
                .as_ref()
                .is_none_or(|original| budget::size(original, 4096) <= 4096)
            && style.bytes <= super::super::MAX_RESOLVED_STYLE_BYTES
        {
            self.resolved = Some((inputs, style.clone()));
        } else {
            self.resolved = None;
        }
    }

    #[cfg(feature = "experimental-view")]
    pub(super) fn view(
        &mut self,
        definition: &'static super::authored::Definition,
        kind: &str,
        truncated: &mut bool,
    ) -> ViewMetadata {
        if let Some((old, view, clipped)) = &self.definition
            && std::ptr::eq(*old, definition)
        {
            *truncated |= *clipped;

            return view.clone();
        }

        let mut clipped = false;
        let view = definition.capture(kind, &mut clipped);

        *truncated |= clipped;
        self.definition = Some((definition, view.clone(), clipped));

        view
    }
}

// Floating-point budgets are fixed-width upper bounds. Scrolling only changes
// these coordinates, so it need not recount the unchanged strings/metadata.
fn same_metadata(a: &super::super::Element, b: &super::super::Element) -> bool {
    a.kind == b.kind
        && a.text == b.text
        && a.text_editable == b.text_editable
        && a.navigation == b.navigation
        && a.semantic_id == b.semantic_id
        && a.source == b.source
        && a.view == b.view
        && a.style_editable == b.style_editable
        && a.style_overrides == b.style_overrides
        && a.accessibility == b.accessibility
        && a.handlers == b.handlers
        && a.tab_id == b.tab_id
        && a.entry_id == b.entry_id
        && a.layout.is_some() == b.layout.is_some()
        && a.content_mask.is_some() == b.content_mask.is_some()
        && match (&a.box_model, &b.box_model) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.padding.is_some() == b.padding.is_some()
                    && a.content.is_some() == b.content.is_some()
                    && a.margin.is_some() == b.margin.is_some()
                    && a.unavailable == b.unavailable
            }
            _ => false,
        }
}

#[cfg(test)]
mod tests {
    //! Covers: cached budgets across geometry/identity changes and metadata invalidation.
    //! Real: element DTOs, budget cache and JSON encoding. Doubles: none; no GPU required.
    use super::*;
    use crate::inspection::{BoxModel, Element, Layout};

    #[test]
    fn cached_budgets_bound_geometry_changes_and_recount_changed_metadata() {
        let mut cache = BoundaryCache::default();

        let mut element: Element = serde_json::from_value(serde_json::json!({
            "id":"element-1-0", "kind":"Div", "metadata_truncated":false
        }))
        .unwrap();

        let mut previous = 0;

        for text in [
            "",
            "é🦀",
            "\0\n\"\\",
            "a long text value with different metadata",
        ] {
            element.text = Some(text.repeat(256));

            for coordinate in [0., -0.01, f32::MIN, f32::MAX] {
                let layout = Layout {
                    x: coordinate,
                    y: -coordinate,
                    width: coordinate.abs(),
                    height: 100.,
                };

                element.layout = Some(layout);
                element.content_mask = Some(layout);
                element.box_model = Some(BoxModel {
                    border: layout,
                    padding: Some(layout),
                    content: Some(layout),
                    margin: None,
                    unavailable: vec!["auto margin".into()],
                });
                element.id = format!("element-{}-{}", u64::MAX, usize::MAX);
                element.parent = Some(element.id.clone());
                element.stable_id = Some(format!("boundary-{}", u64::MAX));
                element.paint_order = Some(usize::MAX);
                let estimate = cache.element_bytes(&element);

                assert!(estimate >= serde_json::to_vec(&element).unwrap().len());
                assert!(
                    estimate > previous,
                    "changed text must invalidate the budget"
                );

                if coordinate == f32::MAX {
                    previous = estimate;
                }
            }
        }
    }
}
