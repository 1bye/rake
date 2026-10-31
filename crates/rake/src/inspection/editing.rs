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
                || (number < 0.
                    && !property.starts_with("margin-")
                    && !matches!(property, "top" | "right" | "bottom" | "left"))
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
            "display" => next.display = Some(keyword(value)?),
            "visibility" => next.visibility = Some(keyword(value)?),
            "position" => next.position = Some(keyword(value)?),
            "flex-direction" => next.flex_direction = Some(keyword(value)?),
            "flex-wrap" => next.flex_wrap = Some(keyword(value)?),
            "align-items" => next.align_items = Some(keyword(value)?),
            "align-self" => next.align_self = Some(keyword(value)?),
            "align-content" => next.align_content = Some(keyword(value)?),
            "justify-content" => next.justify_content = Some(keyword(value)?),
            "flex-grow" | "flex-shrink" => {
                let number: f32 = value.parse().map_err(|_| rejected("invalid flex factor"))?;
                if !number.is_finite() || !(0. ..=100_000.).contains(&number) {
                    return Err(rejected("flex factor must be finite and nonnegative"));
                }
                if property == "flex-grow" {
                    next.flex_grow = Some(number);
                } else {
                    next.flex_shrink = Some(number);
                }
            }
            "flex-basis" => {
                next.flex_basis = Some(if value == "auto" {
                    Length::Auto
                } else {
                    length()?.into()
                })
            }
            "grid-template-columns" => next.grid_cols = Some(grid(value)?),
            "grid-template-rows" => next.grid_rows = Some(grid(value)?),
            "overflow" => {
                let overflow = keyword(value)?;
                next.overflow.x = Some(overflow);
                next.overflow.y = Some(overflow);
            }
            "overflow-x" => next.overflow.x = Some(keyword(value)?),
            "overflow-y" => next.overflow.y = Some(keyword(value)?),
            "border-style" => next.border_style = Some(keyword(value)?),
            "border-width" => {
                let width = Some(length()?);
                next.border_widths.top = width;
                next.border_widths.right = width;
                next.border_widths.bottom = width;
                next.border_widths.left = width;
            }
            "border-top-width" => next.border_widths.top = Some(length()?),
            "border-right-width" => next.border_widths.right = Some(length()?),
            "border-bottom-width" => next.border_widths.bottom = Some(length()?),
            "border-left-width" => next.border_widths.left = Some(length()?),
            "border-radius" => {
                let radius = Some(length()?);
                next.corner_radii.top_left = radius;
                next.corner_radii.top_right = radius;
                next.corner_radii.bottom_left = radius;
                next.corner_radii.bottom_right = radius;
            }
            "border-top-left-radius" => next.corner_radii.top_left = Some(length()?),
            "border-top-right-radius" => next.corner_radii.top_right = Some(length()?),
            "border-bottom-left-radius" => next.corner_radii.bottom_left = Some(length()?),
            "border-bottom-right-radius" => next.corner_radii.bottom_right = Some(length()?),
            "top" | "right" | "bottom" | "left" => {
                let length = Some(if value == "auto" {
                    Length::Auto
                } else {
                    length()?.into()
                });
                match property {
                    "top" => next.inset.top = length,
                    "right" => next.inset.right = length,
                    "bottom" => next.inset.bottom = length,
                    _ => next.inset.left = length,
                }
            }
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
            "gap" => {
                let gap = Some(DefiniteLength::from(length()?));
                next.gap.width = gap;
                next.gap.height = gap;
            }
            _ => {
                return Err(rejected(
                    "unsupported property; see the Elements editable-style coverage",
                ));
            }
        }
        if declaration.enabled {
            gpui::Refineable::refine(&mut style, &next);
        }
    }
    Ok(style)
}

#[cfg(feature = "gpui")]
fn keyword<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, InspectionError> {
    if value.split('-').any(str::is_empty)
        || !value.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
    {
        return Err(rejected("expected a supported CSS keyword"));
    }
    let variant = if value == "nowrap" {
        "NoWrap".into()
    } else {
        value
            .split('-')
            .map(|part| {
                let mut chars = part.chars();
                chars.next().map_or(String::new(), |first| {
                    first.to_ascii_uppercase().to_string() + chars.as_str()
                })
            })
            .collect::<String>()
    };
    serde_json::from_value(serde_json::Value::String(variant))
        .map_err(|_| rejected("keyword is not supported by this GPUI property"))
}

#[cfg(feature = "gpui")]
fn grid(value: &str) -> Result<gpui::GridTemplate, InspectionError> {
    let compact: String = value.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let (count, track) = compact
        .strip_prefix("repeat(")
        .and_then(|s| s.strip_suffix(')'))
        .and_then(|s| s.split_once(','))
        .ok_or_else(|| {
            rejected("grid template must be repeat(N, minmax(0|min-content|max-content, 1fr))")
        })?;
    let repeat: u16 = count
        .parse()
        .map_err(|_| rejected("invalid grid repeat count"))?;
    if !(1..=256).contains(&repeat) {
        return Err(rejected("grid repeat count must be 1–256"));
    }
    let min_size = match track {
        "minmax(0,1fr)" => gpui::GridTemplateMinSize::Zero,
        "minmax(min-content,1fr)" => gpui::GridTemplateMinSize::MinContent,
        "minmax(max-content,1fr)" => gpui::GridTemplateMinSize::MaxContent,
        _ => {
            return Err(rejected(
                "GPUI supports equal fractional grid tracks with 0, min-content or max-content minima",
            ));
        }
    };
    Ok(gpui::GridTemplate { repeat, min_size })
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
            ("display", "inline-flex"),
            ("flex-direction", "column--reverse"),
            ("flex-grow", "NaN"),
            ("flex-shrink", "-1"),
            ("overflow", "auto"),
            ("border-width", "-2px"),
            ("grid-template-columns", "repeat(257, minmax(0, 1fr))"),
            ("grid-template-columns", "repeat(2, 50px)"),
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
                declaration("color", "#11223344"),
                declaration("top", "-2px"),
                declaration("grid-template-rows", "repeat(3, minmax(min-content, 1fr))"),
                declaration("flex-wrap", "nowrap"),
                declaration("align-items", "flex-start"),
                declaration("justify-content", "space-between"),
                declaration("border-style", "dashed"),
                declaration("border-radius", "0.5rem")
            ])
            .is_ok()
        );
    }
}
