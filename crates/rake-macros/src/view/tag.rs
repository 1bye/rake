//! Resolved tag capabilities; inline-only names remain valid inside composed text.

pub(super) const HEADING_CONTENT: &str =
    "headings compose text like p; use a div with role={Role::Heading} for rich content";

pub(super) const PARAGRAPH_CONTENT: &str = "paragraphs accept text, span, strong, em, br, if and for; place layout elements, components and widgets outside p";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tag {
    Container(Option<&'static str>),
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
            "div" | "section" | "span" => Self::Container(None),
            "nav" => Self::Container(Some("Navigation")),
            "header" => Self::Container(Some("Banner")),
            "footer" => Self::Container(Some("ContentInfo")),
            "main" => Self::Container(Some("Main")),
            "aside" => Self::Container(Some("Complementary")),
            "article" => Self::Container(Some("Article")),
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

    pub fn is_inline_name(path: &syn::Path) -> bool {
        ["span", "strong", "em", "br"]
            .iter()
            .any(|name| path.is_ident(name))
    }

    pub fn required_attributes(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Image => &[
                ("src", "img requires src"),
                ("alt", "img requires alt text"),
            ],
            Self::Svg => &[("path", "svg requires path")],
            Self::Entity => &[("entity", "view requires entity={&stored_entity}")],
            _ => &[],
        }
    }

    pub fn leaf_message(self) -> Option<&'static str> {
        match self {
            Self::Image => Some("img cannot have children"),
            Self::Svg => Some("svg cannot have children"),
            Self::Rule => Some("hr cannot have children"),
            Self::Entity => Some("view mounts a retained entity and cannot have children"),
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
