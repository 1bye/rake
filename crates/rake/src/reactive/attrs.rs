use crate::gpui::{
    App, ClickEvent, Refineable, SharedString, StatefulInteractiveElement, StyleRefinement, Styled,
    Window,
};

use super::Event;

#[doc = "Forwarded root styling, accessibility attributes, and click behavior."]
#[derive(Clone, Default)]
pub struct Attrs {
    #[doc = "Additional checked utility styles."]
    pub style: StyleRefinement,
    #[doc = "An accessible label for the root."]
    pub aria_label: Option<SharedString>,
    #[doc = "An accessible description for the root."]
    pub aria_description: Option<SharedString>,
    #[doc = "The root's expanded state."]
    pub aria_expanded: Option<bool>,
    #[doc = "Disables forwarded button interaction."]
    pub disabled: bool,
    #[doc = "A forwarded click handler."]
    pub click: Option<Event<ClickEvent>>,
}

impl Attrs {
    #[doc(hidden)]
    pub fn apply<E: Styled + StatefulInteractiveElement>(
        self,
        mut element: E,
        disabled: bool,
    ) -> E {
        element.style().refine(&self.style);
        if let Some(label) = self.aria_label {
            element = element.aria_label(label);
        }
        if let Some(description) = self.aria_description {
            element = element.aria_description(description);
        }
        if let Some(expanded) = self.aria_expanded {
            element = element.aria_expanded(expanded);
        }
        if !disabled && let Some(click) = self.click {
            element = element.on_click(move |event, window, cx| click.call(event, window, cx));
        }
        element
    }

    #[doc = "Sets the forwarded click handler."]
    pub fn on_click(&mut self, callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) {
        self.click = Some(Event::new(callback));
    }
}
