//! Covers: component props, children, slots, callbacks, and single-evaluation ownership.
//! Real: public macro expansion, GPUI RenderOnce components, and accessible text conversion.
//! Doubles: none.
//! Requires: experimental-view; no display or GPU.

#![cfg(feature = "experimental-view")]

use std::cell::RefCell;
use std::rc::Rc;

use rake::gpui::{
    self, AnyElement, Element, ElementId, InteractiveElement, IntoElement, ParentElement,
    RenderOnce,
};

#[derive(Debug, PartialEq)]
enum Tone {
    Quiet,
    Highlighted,
}

#[derive(IntoElement)]
struct Badge {
    id: ElementId,
    label: String,
    tone: Tone,
    on_activate: Box<dyn Fn()>,
}

impl RenderOnce for Badge {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        rake::view! {
            <button id={self.id} aria-label={self.label.clone()}
                style={|s| s.opacity(if self.tone == Tone::Quiet { 0.5 } else { 1. })}
                on:click={move |_, _, _| (self.on_activate)()}>
                {self.label}
            </button>
        }
    }
}

mod ui {
    use super::*;

    #[derive(IntoElement)]
    pub(super) struct Card {
        pub id: ElementId,
        pub header: AnyElement,
        pub footer: Option<AnyElement>,
        pub children: Vec<AnyElement>,
    }

    impl RenderOnce for Card {
        fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
            gpui::div()
                .id(self.id)
                .child(self.header)
                .children(self.children)
                .children(self.footer)
        }
    }
}

#[test]
fn properties_keep_their_types_and_move_owned_values_once() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let activated = calls.clone();
    let label = String::from("Experimental");

    let badge = rake::view! {
        <Badge id={"status".into()} label={label} tone={Tone::Highlighted}
            on_activate={Box::new(move || activated.borrow_mut().push("activated"))}/>
    };

    assert_eq!(badge.id, ElementId::from("status"));
    assert_eq!(badge.label, "Experimental");
    assert_eq!(badge.tone, Tone::Highlighted);
    (badge.on_activate)();
    assert_eq!(*calls.borrow(), ["activated"]);
}

#[test]
fn paired_components_collect_selected_children_and_typed_slots_in_order() {
    let visited = RefCell::new(Vec::new());

    let mut card = rake::view! {
        <ui::Card
            id={ { visited.borrow_mut().push(0); "card".into() } }
            header={ { visited.borrow_mut().push(1); gpui::div().id("header").into_any_element() } }
            footer={ { visited.borrow_mut().push(2); None } }>
            { { visited.borrow_mut().push(3); "Body" } }
            if false {
                {unreachable_child()}
            } else {
                <p id="selected">{ { visited.borrow_mut().push(4); "Selected" } }</p>
            }
            for index in 0..2usize {
                <div id={("row", index)}>{ { visited.borrow_mut().push(5 + index); "Row" } }</div>
            }
        </ui::Card>
    };

    assert_eq!(*visited.borrow(), [0, 1, 2, 3, 4, 5, 6]);
    assert_eq!(card.id, ElementId::from("card"));
    assert_eq!(card.children.len(), 4);
    assert!(card.footer.is_none());

    let text = card.children[0]
        .downcast_mut::<gpui::Text>()
        .expect("accessible child text");

    let mut info = gpui::accesskit::Node::new(gpui::accesskit::Role::Label);

    text.write_a11y_info(&mut info);

    assert_eq!(info.value(), Some("Body"));
    assert!(
        card.header
            .downcast_mut::<gpui::Stateful<gpui::Div>>()
            .is_some()
    );
}

#[test]
fn empty_paired_tags_and_explicit_children_accept_empty_vectors() {
    let paired = rake::view! {
        <ui::Card id={"paired".into()} header={gpui::div().into_any_element()} footer={None}></ui::Card>
    };

    let explicit = rake::view! {
        <ui::Card id={"explicit".into()} header={gpui::div().into_any_element()} footer={None}
            children={Vec::new()}/>
    };

    assert!(paired.children.is_empty());
    assert!(explicit.children.is_empty());
}

#[cfg(debug_assertions)]
#[test]
fn duplicate_authored_children_are_diagnosed_before_component_rendering() {
    let duplicate = std::panic::catch_unwind(|| {
        rake::view! {
            <ui::Card id={"duplicate".into()} header={gpui::div().into_any_element()} footer={None}>
                <p id="same">"First"</p>
                <p id="same">"Second"</p>
            </ui::Card>
        }
    });

    assert!(duplicate.is_err());
}

fn unreachable_child() -> &'static str {
    panic!("untaken component child")
}
