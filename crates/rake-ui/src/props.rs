#[doc(hidden)]
pub struct PropSet;

#[doc(hidden)]
pub struct PropMissing;

#[doc(hidden)]
pub trait PropsBuilder: Sized {
    type Builder;
    type EmptyBuilder;
    type FilledBuilder;

    fn builder() -> Self::Builder;

    fn builder_empty() -> Self::EmptyBuilder;

    fn builder_from(props: Self) -> Self::FilledBuilder;
}

#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "missing required component property `{Name}`",
    label = "set the required property before calling build"
)]
pub trait RequiredProp<Name> {}

impl<Name> RequiredProp<Name> for PropSet {}

#[doc = "Resolves a plain or reactive attribute through the current application."]
pub trait Value<T> {
    #[doc = "Reads the current attribute value."]
    fn resolve(self, cx: &crate::gpui::App) -> T;
}

impl<T> Value<T> for T {
    fn resolve(self, _: &crate::gpui::App) -> T {
        self
    }
}

#[doc(hidden)]
pub struct PropComparison<'a, T>(pub &'a T, pub &'a T);

#[doc(hidden)]
pub trait EquivalentProp {
    fn equivalent(&self) -> bool;
}

impl<T: PartialEq> EquivalentProp for PropComparison<'_, T> {
    fn equivalent(&self) -> bool {
        self.0 == self.1
    }
}

impl<T> EquivalentProp for &PropComparison<'_, T> {
    fn equivalent(&self) -> bool {
        false
    }
}

/// Resolves an attribute with a GPUI conversion.
#[doc(hidden)]
pub trait IntoAttribute<T> {
    /// Reads and converts the current value.
    fn resolve_into(self, cx: &crate::gpui::App) -> T;
}

impl<T, U: Into<T>> IntoAttribute<T> for U {
    fn resolve_into(self, _: &crate::gpui::App) -> T {
        self.into()
    }
}

/// Resolves a paragraph run while preserving ordinary borrowed strings.
#[doc(hidden)]
pub trait ParagraphTextValue {
    /// The resolved text representation.
    type Text: AsRef<str>;

    /// Reads the current text run.
    fn resolve_text(self, cx: &crate::gpui::App) -> Self::Text;
}

impl<T: AsRef<str>> ParagraphTextValue for T {
    type Text = T;

    fn resolve_text(self, _: &crate::gpui::App) -> Self::Text {
        self
    }
}
