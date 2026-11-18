use std::rc::Rc;

use crate::gpui::{
    App, ClickEvent, Interactivity, Refineable, SharedString, StatefulInteractiveElement,
    StyleRefinement, Styled, Window,
};

use super::Event;

type RootListener = (&'static str, Rc<dyn Fn(&mut Interactivity)>);

#[doc = "Forwarded root styling, accessibility attributes, and event handlers."]
#[derive(Clone, Default)]
pub struct Attrs {
    #[doc = "Additional checked utility styles."]
    pub style: StyleRefinement,
    #[doc = "An accessible label for the root."]
    pub aria_label: Option<SharedString>,
    #[doc = "An accessible description for the root."]
    pub aria_description: Option<SharedString>,
    #[doc = "Keyboard shortcuts for the root."]
    pub aria_keyshortcuts: Option<SharedString>,
    #[doc = "The root's expanded state."]
    pub aria_expanded: Option<bool>,
    #[doc = "Disables forwarded button interaction."]
    pub disabled: bool,
    #[doc = "A forwarded click handler."]
    pub click: Option<Event<ClickEvent>>,
    #[doc(hidden)]
    pub listeners: Vec<RootListener>,
}

impl Attrs {
    #[doc(hidden)]
    pub fn apply<E: Styled + StatefulInteractiveElement>(
        &self,
        mut element: E,
        disabled: bool,
        hover: bool,
        drag: bool,
    ) -> E {
        element.style().refine(&self.style);
        if let Some(label) = self.aria_label.clone() {
            element = element.aria_label(label);
        }
        if let Some(description) = self.aria_description.clone() {
            element = element.aria_description(description);
        }
        if let Some(expanded) = self.aria_expanded {
            element = element.aria_expanded(expanded);
        }
        if let Some(keyshortcuts) = self.aria_keyshortcuts.clone() {
            element = element.aria_keyshortcuts(keyshortcuts);
        }
        if !disabled {
            if let Some(click) = self.click.clone() {
                element = element.on_click(move |event, window, cx| click.call(event, window, cx));
            }
            for (name, listener) in &self.listeners {
                if (*name != "on_hover" || hover) && (*name != "on_drag" || drag) {
                    listener(element.interactivity());
                }
            }
        }
        element
    }

    #[doc(hidden)]
    pub fn push_listener(
        &mut self,
        name: &'static str,
        listener: impl Fn(&mut Interactivity) + 'static,
    ) {
        if matches!(name, "on_hover" | "on_drag") {
            self.listeners.retain(|(previous, _)| *previous != name);
        }
        self.listeners.push((name, Rc::new(listener)));
    }

    #[doc = "Sets the forwarded click handler."]
    pub fn on_click(&mut self, callback: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) {
        self.click = Some(Event::new(callback));
    }
}
