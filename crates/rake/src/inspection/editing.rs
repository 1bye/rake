//! Bounded temporary base refinements owned by a stable inspected boundary lifetime.

use super::{InspectionError, Store, StyleDeclaration, StyleEdit};

#[derive(Default)]
pub(super) struct History {
    pub current: Vec<StyleDeclaration>,
    undo: Vec<Vec<StyleDeclaration>>,
}

impl Store {
    pub(crate) fn edit_style(
        &self,
        id: &str,
        generation: &str,
        edit: &StyleEdit,
    ) -> Result<(), InspectionError> {
        let mut state = self.0.borrow_mut();
        let element = state
            .windows
            .values()
            .find(|host| host.snapshot.capture == generation)
            .ok_or(InspectionError::StaleGeneration)?
            .snapshot
            .elements
            .iter()
            .find(|element| element.id == id)
            .ok_or(InspectionError::NotRetained)?;
        let stable = element
            .stable_id
            .clone()
            .filter(|_| element.style_editable)
            .ok_or_else(|| rejected("boundary has no editable stable style"))?;
        if state.edits.len() >= 64 && !state.edits.contains_key(&stable) {
            return Err(rejected("64 edited boundaries already retained"));
        }
        let mut next = state
            .edits
            .get(&stable)
            .map(|h| h.current.clone())
            .unwrap_or_default();
        match edit {
            StyleEdit::Replace { declarations } => next.clone_from(declarations),
            StyleEdit::Set { declaration } => {
                next.retain(|old| old.property != declaration.property);
                next.push(declaration.clone());
            }
            StyleEdit::Reset => next.clear(),
            StyleEdit::Undo => {
                let history = state
                    .edits
                    .get_mut(&stable)
                    .ok_or_else(|| rejected("no previous edit"))?;
                history.current = history
                    .undo
                    .pop()
                    .ok_or_else(|| rejected("no previous edit"))?;
                state.changed.notify(usize::MAX);
                return Ok(());
            }
        }
        validate(&next)?;
        let history = state.edits.entry(stable).or_default();
        if next != history.current {
            if history.undo.len() == 32 {
                history.undo.remove(0);
            }
            history
                .undo
                .push(std::mem::replace(&mut history.current, next));
        }
        state.changed.notify(usize::MAX);
        Ok(())
    }
}

fn rejected(message: &str) -> InspectionError {
    InspectionError::StyleEdit(message.into())
}

fn validate(declarations: &[StyleDeclaration]) -> Result<(), InspectionError> {
    if declarations.len() > 32 {
        return Err(rejected("at most 32 declarations are allowed"));
    }
    #[cfg(not(feature = "gpui"))]
    if !declarations.is_empty() {
        return Err(rejected("style editing needs a GPUI host"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for declaration in declarations {
        if !seen.insert(&declaration.property) {
            return Err(rejected("duplicate property"));
        }
        if declaration.property.len() > 64 || declaration.value.len() > 128 {
            return Err(rejected("declaration exceeds length limit"));
        }
        #[cfg(feature = "gpui")]
        refinement(std::slice::from_ref(declaration))?;
    }
    Ok(())
}

#[cfg(feature = "gpui")]
pub(super) fn refinement(
    declarations: &[StyleDeclaration],
) -> Result<gpui::StyleRefinement, InspectionError> {
    use gpui::{AbsoluteLength, DefiniteLength, Length, StyleRefinement};
    let mut style = StyleRefinement::default();
    for declaration in declarations {
        let value = declaration.value.trim();
        let property = declaration.property.as_str();
        // Validate disabled values too; enabling a declaration must never bypass validation.
        let length = || -> Result<AbsoluteLength, InspectionError> {
            let (number, rem) = if let Some(number) = value.strip_suffix("px") {
                (number, false)
            } else if let Some(number) = value.strip_suffix("rem") {
                (number, true)
            } else if value == "0" {
                ("0", false)
            } else {
                return Err(rejected("length must use px or rem"));
            };
            let number: f32 = number.parse().map_err(|_| rejected("invalid length"))?;
            if !number.is_finite()
                || number.abs() > 100_000.
                || (number < 0. && !property.starts_with("margin-"))
            {
                return Err(rejected("length outside supported range"));
            }
            Ok(if rem {
                gpui::rems(number).into()
            } else {
                gpui::px(number).into()
            })
        };
        let color = || -> Result<gpui::Hsla, InspectionError> {
            let number = if value == "transparent" {
                0
            } else {
                let digits = value
                    .strip_prefix('#')
                    .ok_or_else(|| rejected("color must be #rrggbb or #rrggbbaa"))?;
                if !matches!(digits.len(), 6 | 8)
                    || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(rejected("invalid hex color"));
                }
                let number =
                    u32::from_str_radix(digits, 16).map_err(|_| rejected("invalid hex color"))?;
                if digits.len() == 6 {
                    (number << 8) | 255
                } else {
                    number
                }
            };
            Ok(gpui::rgb_to_hsla(gpui::rgba(number)))
        };
        let mut next = StyleRefinement::default();
        match property {
            "color" => next.text.color = Some(color()?),
            "background-color" => next.background = Some(color()?.into()),
            "border-color" => next.border_color = Some(color()?),
            "opacity" => {
                let opacity: f32 = value.parse().map_err(|_| rejected("invalid opacity"))?;
                if !(0. ..=1.).contains(&opacity) {
                    return Err(rejected("opacity must be 0–1"));
                }
                next.opacity = Some(opacity);
            }
            "font-size" => next.text.font_size = Some(length()?),
            "font-weight" => {
                let weight: u16 = value.parse().map_err(|_| rejected("invalid font weight"))?;
                if !(100..=900).contains(&weight) {
                    return Err(rejected("font weight must be 100–900"));
                }
                next.text.font_weight = Some(gpui::FontWeight(weight as f32));
            }
            "font-family" => {
                if value.is_empty()
                    || !value
                        .chars()
                        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
                {
                    return Err(rejected("font-family must be one literal family name"));
                }
                next.text.font_family = Some(value.to_owned().into());
            }
            "width" | "height" | "min-width" | "min-height" | "max-width" | "max-height" => {
                let size = match property {
                    "min-width" | "min-height" => &mut next.min_size,
                    "max-width" | "max-height" => &mut next.max_size,
                    _ => &mut next.size,
                };
                let length = if value == "auto" {
                    Length::Auto
                } else {
                    Length::from(length()?)
                };
                if property.ends_with("width") {
                    size.width = Some(length);
                } else {
                    size.height = Some(length);
                }
            }
            "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => {
                let length = Some(DefiniteLength::from(length()?));
                match property {
                    "padding-top" => next.padding.top = length,
                    "padding-right" => next.padding.right = length,
                    "padding-bottom" => next.padding.bottom = length,
                    _ => next.padding.left = length,
                }
            }
            "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => {
                let length = Some(Length::from(length()?));
                match property {
                    "margin-top" => next.margin.top = length,
                    "margin-right" => next.margin.right = length,
                    "margin-bottom" => next.margin.bottom = length,
                    _ => next.margin.left = length,
                }
            }
            "row-gap" => next.gap.height = Some(length()?.into()),
            "column-gap" => next.gap.width = Some(length()?.into()),
            _ => {
                return Err(rejected(
                    "unsupported property; use colors, px/rem sizes/spacing, opacity or typography",
                ));
            }
        }
        if declaration.enabled {
            gpui::Refineable::refine(&mut style, &next);
        }
    }
    Ok(style)
}

#[cfg(all(test, feature = "gpui"))]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_disabled_values_duplicates_and_bounds() {
        let declaration = |property: &str, value: &str| StyleDeclaration {
            property: property.into(),
            value: value.into(),
            enabled: false,
        };
        for (property, value) in [
            ("width", "NaNpx"),
            ("height", "-2px"),
            ("opacity", "2"),
            ("color", "url(secret)"),
            ("font-family", "x; color:red"),
            ("unsupported", "10px"),
        ] {
            assert!(
                validate(&[declaration(property, value)]).is_err(),
                "{property}: {value}"
            );
        }
        assert!(
            validate(&[
                declaration("color", "#ffffff"),
                declaration("color", "#000000")
            ])
            .is_err()
        );
        assert!(validate(&vec![declaration("width", "1px"); 33]).is_err());
        assert!(
            validate(&[
                declaration("margin-left", "-2rem"),
                declaration("color", "#11223344")
            ])
            .is_ok()
        );
    }
}
