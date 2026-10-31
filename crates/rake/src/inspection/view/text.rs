//! Replaces supported owned text before layout, restoring the original on each draw.

use gpui::SharedString;
use std::any::Any;

pub(super) enum Original {
    Text(SharedString),
    #[cfg(feature = "experimental-view")]
    Paragraph(rake_ui::view::ParagraphText),
}

pub(super) fn editable(value: &dyn Any) -> bool {
    if value.is::<gpui::Text>() || value.is::<SharedString>() {
        return true;
    }
    #[cfg(feature = "experimental-view")]
    if value.is::<rake_ui::view::ParagraphText>() {
        return true;
    }
    false
}

pub(super) fn replace(value: &mut dyn Any, replacement: &str) -> Option<Box<Original>> {
    let replacement: SharedString = replacement.to_owned().into();
    if let Some(text) = value.downcast_mut::<gpui::Text>() {
        return Some(Box::new(Original::Text(std::mem::replace(
            &mut **text,
            replacement,
        ))));
    }
    if let Some(text) = value.downcast_mut::<SharedString>() {
        return Some(Box::new(Original::Text(std::mem::replace(
            text,
            replacement,
        ))));
    }
    #[cfg(feature = "experimental-view")]
    if let Some(text) = value.downcast_mut::<rake_ui::view::ParagraphText>() {
        let mut content = rake_ui::view::ParagraphContent::default();
        content.push(&replacement, &rake_ui::view::RunStyle::default());
        return Some(Box::new(Original::Paragraph(std::mem::replace(
            text,
            content.finish(),
        ))));
    }
    None
}

pub(super) fn restore(value: &mut dyn Any, original: Original) {
    match original {
        Original::Text(original) => {
            if let Some(text) = value.downcast_mut::<gpui::Text>() {
                **text = original;
            } else if let Some(text) = value.downcast_mut::<SharedString>() {
                *text = original;
            }
        }
        #[cfg(feature = "experimental-view")]
        Original::Paragraph(original) => {
            if let Some(text) = value.downcast_mut::<rake_ui::view::ParagraphText>() {
                *text = original;
            }
        }
    }
}
