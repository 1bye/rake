//! Covers: public view expansion, real GPUI styles/semantics, and control-flow evaluation.
//! Real: macro expansion and GPUI element construction.
//! Doubles: none.
//! Requires: experimental-view; no display/GPU execution.

#![cfg(feature = "experimental-view")]

use std::cell::RefCell;

use rake::gpui::{Element, Styled, accesskit, div, px, rgb};

#[test]
fn builds_gpui_styles_and_evaluates_each_selected_child_once_in_order() {
    let visited = RefCell::new(Vec::new());

    let mut view = rake::view! {
        <div id="root" class="flex flex-col gap-2 p-4"
            style={|s| s.bg(rgb(0x18232b)).p(px(7.0))}>
            { { visited.borrow_mut().push(0); div() } }
            if false {
                {unreachable_child()}
            } else if true {
                <p>{ { visited.borrow_mut().push(1); "selected" } }</p>
            } else {
                {unreachable_child()}
            }
            for index in 2..5usize {
                <div id={("row", index)}>
                    { { visited.borrow_mut().push(index); format!("Row {index}") } }
                </div>
            }
            if let Some(last) = Some(5) {
                { { visited.borrow_mut().push(last); "last" } }
            }
        </div>
    };

    let mut expected = div().flex().flex_col().gap_2().p(px(7.0)).bg(rgb(0x18232b));

    assert_eq!(*visited.borrow(), [0, 1, 2, 3, 4, 5]);
    assert_eq!(Element::id(&view), Some("root".into()));
    assert_eq!(
        format!("{:?}", view.style()),
        format!("{:?}", expected.style())
    );
}

#[test]
fn maps_headings_buttons_lists_and_images_to_accessibility_roles() {
    let heading = rake::view! { <h2 aria-label="Heading"/> };

    let button = rake::view! {
        <button aria-expanded={false} onclick={|_, _, _| {}}>"Save"</button>
    };

    let list = rake::view! { <ul/> };
    let item = rake::view! { <li/> };

    let image = rake::view! {
        <img src="icon.png" alt="Application icon" aria-description="Preview"
            style={|s| s.object_fit(rake::gpui::ObjectFit::Cover)}/>
    };

    let decorative = rake::view! { <img src="texture.png" alt=""/> };
    let mut heading_info = accesskit::Node::new(accesskit::Role::Heading);
    let mut image_info = accesskit::Node::new(accesskit::Role::Image);
    let mut button_info = accesskit::Node::new(accesskit::Role::Button);

    heading.write_a11y_info(&mut heading_info);
    image.write_a11y_info(&mut image_info);
    button.write_a11y_info(&mut button_info);

    assert_eq!(heading.a11y_role(), Some(accesskit::Role::Heading));
    assert_eq!(heading_info.level(), Some(2));
    assert_eq!(heading_info.label(), Some("Heading"));
    assert_eq!(button.a11y_role(), Some(accesskit::Role::Button));
    assert_eq!(button_info.label(), Some("Save"));
    assert_eq!(button_info.is_expanded(), Some(false));
    assert_eq!(list.a11y_role(), Some(accesskit::Role::List));
    assert_eq!(item.a11y_role(), Some(accesskit::Role::ListItem));
    assert_eq!(image.a11y_role(), Some(accesskit::Role::Image));
    assert_eq!(image_info.label(), Some("Application icon"));
    assert_eq!(image_info.description(), Some("Preview"));
    assert_eq!(decorative.a11y_role(), None);
}

#[test]
fn empty_loops_do_not_evaluate_children_and_distinct_sites_have_distinct_ids() {
    let first = rake::view! { <div/> };
    let second = rake::view! { <div/> };

    let _empty = rake::view! {
        <div>
            for index in std::iter::empty::<usize>() {
                <p id={index}>{unreachable_child()}</p>
            }
        </div>
    };

    assert_ne!(Element::id(&first), Element::id(&second));
}

fn unreachable_child() -> &'static str {
    panic!("empty-loop child evaluated")
}

#[test]
fn event_handlers_and_styles_use_gpui_types_without_a_caller_prelude() {
    let __rake_view_element_0 = "Caller binding";

    let _view = rake::view! {
        <section class="grid grid-cols-2 p-2 font-medium">
            <button id="save" class="rounded border px-4 py-2 text-sm"
                onclick={|event, _, _| { let _: &rake::gpui::ClickEvent = event; }}
                onkeydown={|event, _, _| { let _: &rake::gpui::KeyDownEvent = event; }}
                onkeyup={|_, _, _| {}}
                onmousedown={|_, _, _| {}}
                onmouseup={|_, _, _| {}}
                onmousemove={|_, _, _| {}}>
                "Save"
            </button>
            <span>{__rake_view_element_0}</span>
        </section>
    };
}

#[test]
fn all_advertised_utilities_resolve_to_the_pinned_gpui_api() {
    let _view = rake::view! {
        <div class="flex block grid hidden flex-col flex-row flex-wrap flex-1
            items-start items-center items-end justify-start justify-center justify-end justify-between
            w-full h-full size-full min-w-0 min-h-0 overflow-hidden overflow-scroll overflow-x-scroll overflow-y-scroll
            rounded rounded-none rounded-sm rounded-md rounded-lg rounded-xl rounded-full
            border border-0 border-1 border-2 shadow-sm shadow-md shadow-lg cursor-pointer
            text-xs text-sm text-base text-lg text-xl text-2xl text-3xl text-left text-center text-right
            font-normal font-medium font-semibold font-bold grid-cols-1 grid-cols-12
            p-0 p-1 p-2 p-3 p-4 p-5 p-6 p-7 p-8 p-12 p-16 p-20 p-24 p-32
            px-1 py-1 pt-1 pb-1 pl-1 pr-1 m-1 mx-1 my-1 mt-1 mb-1 ml-1 mr-1 gap-1"/>
    };
}

#[cfg(debug_assertions)]
#[test]
fn duplicate_dynamic_sibling_ids_fail_before_layout() {
    let duplicate = std::panic::catch_unwind(|| {
        rake::view! {
            <div>
                for key in [7usize, 7] {
                    <button id={key}>"Repeated"</button>
                }
            </div>
        }
    });

    assert!(
        duplicate.is_err(),
        "duplicate sibling IDs must be diagnosed"
    );
}

#[test]
fn child_identity_scopes_are_local_and_only_rendered_branches_are_checked() {
    let _tree = rake::view! {
        <div>
            <section id="first"><button id="save">"Save"</button></section>
            <section id="second"><button id="save">"Save"</button></section>
            if true {
                <p id="status">"Visible"</p>
            } else {
                <p id="status">"Hidden"</p>
            }
        </div>
    };
}

#[cfg(debug_assertions)]
#[test]
fn duplicate_ids_in_embedded_gpui_roots_are_also_diagnosed() {
    let duplicate = std::panic::catch_unwind(|| {
        rake::view! {
            <div>
                <p id="status">"First"</p>
                {div().id("status")}
            </div>
        }
    });

    let panic = duplicate.err().expect("duplicate embedded root must fail");
    let message = panic.downcast_ref::<String>().expect("diagnostic");

    assert!(message.contains("duplicate sibling id"));
    assert!(message.contains("status"));
}

#[test]
fn disabled_buttons_expose_their_state_without_accessibility_activation_actions() {
    let calls = std::cell::Cell::new(0);

    let button = rake::view! {
        <button disabled={ { calls.set(calls.get() + 1); true } }
            aria-label="Save document" aria-description="Unavailable while saving"
            aria-keyshortcuts="Control+S" onclick={|_, _, _| {}}>
            "Save"
        </button>
    };

    let mut info = accesskit::Node::new(button.a11y_role().expect("button role"));

    button.write_a11y_info(&mut info);

    assert_eq!(calls.get(), 1);
    assert!(info.is_disabled());
    assert!(!info.supports_action(accesskit::Action::Click));
    assert!(!info.supports_action(accesskit::Action::Focus));
    assert_eq!(info.label(), Some("Save document"));
    assert_eq!(info.description(), Some("Unavailable while saving"));
    assert_eq!(info.keyboard_shortcut(), Some("Control+S"));
}
