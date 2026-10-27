//! Resolved tag capabilities; inline-only names remain valid inside composed text.

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tag {
    Container,
    Paragraph,
    Heading(u8),
    List,
    ListItem,
    Button,
    Image,
    Svg,
    Rule,
    Entity,
    Component,
    Inline,
    Unsupported,
}

pub(super) fn display_name(path: &syn::Path) -> String {
    let name = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::");

    if path.leading_colon.is_some() {
        format!("::{name}")
    } else {
        name
    }
}

impl Tag {
    pub fn resolve(path: &syn::Path) -> Self {
        if path
            .segments
            .last()
            .is_some_and(|segment| segment.ident.to_string().starts_with(char::is_uppercase))
        {
            return Self::Component;
        }

        match display_name(path).as_str() {
            "div" | "section" | "span" | "nav" | "header" | "footer" | "main" | "aside"
            | "article" => Self::Container,
            "p" => Self::Paragraph,
            "h1" => Self::Heading(1),
            "h2" => Self::Heading(2),
            "h3" => Self::Heading(3),
            "h4" => Self::Heading(4),
            "h5" => Self::Heading(5),
            "h6" => Self::Heading(6),
            "ul" | "ol" => Self::List,
            "li" => Self::ListItem,
            "button" => Self::Button,
            "img" => Self::Image,
            "svg" => Self::Svg,
            "hr" => Self::Rule,
            "view" => Self::Entity,
            "strong" | "em" | "br" => Self::Inline,
            _ => Self::Unsupported,
        }
    }

    pub fn semantic_role(self, name: &str) -> Option<&'static str> {
        match name {
            "nav" => Some("Navigation"),
            "header" => Some("Banner"),
            "footer" => Some("ContentInfo"),
            "main" => Some("Main"),
            "aside" => Some("Complementary"),
            "article" => Some("Article"),
            _ => None,
        }
    }

    pub fn composes_text(self) -> bool {
        matches!(self, Self::Paragraph | Self::Heading(_))
    }

    pub fn supports_disabled(self) -> bool {
        self == Self::Button
    }

    pub fn auto_label(self) -> bool {
        self == Self::Button
    }
}
