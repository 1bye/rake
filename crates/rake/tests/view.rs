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
        <button aria-expanded={false} on:click={|_, _, _| {}}>"Save"</button>
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
                on:click={|event, _, _| { let _: &rake::gpui::ClickEvent = event; }}
                on:key-down={|event, _, _| { let _: &rake::gpui::KeyDownEvent = event; }}
                on:key-up={|_, _, _| {}}
                on:mouse-down={|_, _, _| {}}
                on:mouse-up={|_, _, _| {}}
                on:mouse-move={|_, _, _| {}}>
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
            aria-keyshortcuts="Control+S" on:click={|_, _, _| {}}>
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

#[test]
fn every_supported_utility_compiles_against_gpui() {
    let _ = rake::view! { <div class="flex block grid hidden flex-col flex-row flex-wrap flex-1 items-start items-center items-end justify-start justify-center justify-end justify-between w-full h-full size-full min-w-0 min-h-0 overflow-hidden overflow-scroll overflow-x-scroll overflow-y-scroll rounded-none rounded-sm rounded-md rounded-lg rounded-xl rounded-full border-0 border-1"/> };
    let _ = rake::view! { <div class="border-2 shadow-sm shadow-md shadow-lg cursor-pointer text-xs text-sm text-base text-lg text-xl text-2xl text-3xl text-left text-center text-right rounded border shrink-0 grow grow-0 flex-none relative absolute items-stretch self-start self-center self-end justify-around truncate whitespace-nowrap overflow-x-hidden overflow-y-hidden"/> };
    let _ = rake::view! { <div class="border-t border-t-0 border-t-1 border-t-2 border-b border-b-0 border-b-1 border-b-2 border-l border-l-0 border-l-1 border-l-2 border-r border-r-0 border-r-1 border-r-2 font-normal font-medium font-semibold font-bold grid-cols-1 grid-cols-2 grid-cols-3 grid-cols-4 grid-cols-5 grid-cols-6 grid-cols-7 grid-cols-8 grid-cols-9 grid-cols-10 grid-cols-11 grid-cols-12"/> };
    let _ = rake::view! { <div class="p-0 p-1 p-2 p-3 p-4 p-5 p-6 p-7 p-8 p-12 p-16 p-20 p-24 p-32 px-0 px-1 px-2 px-3 px-4 px-5 px-6 px-7 px-8 px-12 px-16 px-20 px-24 px-32 py-0 py-1 py-2 py-3"/> };
    let _ = rake::view! { <div class="py-4 py-5 py-6 py-7 py-8 py-12 py-16 py-20 py-24 py-32 pt-0 pt-1 pt-2 pt-3 pt-4 pt-5 pt-6 pt-7 pt-8 pt-12 pt-16 pt-20 pt-24 pt-32 pb-0 pb-1 pb-2 pb-3 pb-4 pb-5 pb-6 pb-7"/> };
    let _ = rake::view! { <div class="pb-8 pb-12 pb-16 pb-20 pb-24 pb-32 pl-0 pl-1 pl-2 pl-3 pl-4 pl-5 pl-6 pl-7 pl-8 pl-12 pl-16 pl-20 pl-24 pl-32 pr-0 pr-1 pr-2 pr-3 pr-4 pr-5 pr-6 pr-7 pr-8 pr-12 pr-16 pr-20"/> };
    let _ = rake::view! { <div class="pr-24 pr-32 m-0 m-1 m-2 m-3 m-4 m-5 m-6 m-7 m-8 m-12 m-16 m-20 m-24 m-32 mx-0 mx-1 mx-2 mx-3 mx-4 mx-5 mx-6 mx-7 mx-8 mx-12 mx-16 mx-20 mx-24 mx-32 my-0 my-1"/> };
    let _ = rake::view! { <div class="my-2 my-3 my-4 my-5 my-6 my-7 my-8 my-12 my-16 my-20 my-24 my-32 mt-0 mt-1 mt-2 mt-3 mt-4 mt-5 mt-6 mt-7 mt-8 mt-12 mt-16 mt-20 mt-24 mt-32 mb-0 mb-1 mb-2 mb-3 mb-4 mb-5"/> };
    let _ = rake::view! { <div class="mb-6 mb-7 mb-8 mb-12 mb-16 mb-20 mb-24 mb-32 ml-0 ml-1 ml-2 ml-3 ml-4 ml-5 ml-6 ml-7 ml-8 ml-12 ml-16 ml-20 ml-24 ml-32 mr-0 mr-1 mr-2 mr-3 mr-4 mr-5 mr-6 mr-7 mr-8 mr-12"/> };
    let _ = rake::view! { <div class="mr-16 mr-20 mr-24 mr-32 gap-0 gap-1 gap-2 gap-3 gap-4 gap-5 gap-6 gap-7 gap-8 gap-12 gap-16 gap-20 gap-24 gap-32 gap-x-0 gap-x-1 gap-x-2 gap-x-3 gap-x-4 gap-x-5 gap-x-6 gap-x-7 gap-x-8 gap-x-12 gap-x-16 gap-x-20 gap-x-24 gap-x-32"/> };
    let _ = rake::view! { <div class="gap-y-0 gap-y-1 gap-y-2 gap-y-3 gap-y-4 gap-y-5 gap-y-6 gap-y-7 gap-y-8 gap-y-12 gap-y-16 gap-y-20 gap-y-24 gap-y-32 w-0 w-1 w-2 w-3 w-4 w-5 w-6 w-7 w-8 w-12 w-16 w-20 w-24 w-32 h-0 h-1 h-2 h-3"/> };
    let _ = rake::view! { <div class="h-4 h-5 h-6 h-7 h-8 h-12 h-16 h-20 h-24 h-32 size-0 size-1 size-2 size-3 size-4 size-5 size-6 size-7 size-8 size-12 size-16 size-20 size-24 size-32"/> };
}

#[test]
fn conditional_classes_apply_once_in_attribute_order() {
    let visits = std::cell::Cell::new(0);

    let mut view = rake::view! {
        <div class="shrink-0"
            class:grow={ { visits.set(visits.get() + 1); true } }
            class:grow-0={false}
            class:font-bold={false}
            style={|s| s.p_4()}/>
    };

    let mut expected = div().flex().flex_col().flex_shrink_0().flex_grow_1().p_4();

    assert_eq!(visits.get(), 1);
    assert_eq!(
        format!("{:?}", view.style()),
        format!("{:?}", expected.style())
    );
}

struct DragPreview;

impl rake::gpui::Render for DragPreview {
    fn render(
        &mut self,
        _: &mut rake::gpui::Window,
        _: &mut rake::gpui::Context<Self>,
    ) -> impl rake::gpui::IntoElement {
        rake::gpui::Empty
    }
}

fn drag_preview(
    _: &usize,
    _: rake::gpui::Point<rake::gpui::Pixels>,
    _: &mut rake::gpui::Window,
    cx: &mut rake::gpui::App,
) -> rake::gpui::Entity<DragPreview> {
    use rake::gpui::AppContext;

    cx.new(|_| DragPreview)
}

#[test]
fn every_canonical_event_compiles_against_the_gpui_signature() {
    use rake::gpui::AppContext;

    let _ = rake::view! {
        <div on:click={|_, _, _| {}}
            on:key-down={|_, _, _| {}} on:key-up={|_, _, _| {}}
            on:mouse-move={|_, _, _| {}}
            on:mouse-down={|_, _, _| {}} on:mouse-up={|_, _, _| {}}
            on:right-mouse-down={|_, _, _| {}} on:right-mouse-up={|_, _, _| {}}
            on:middle-mouse-down={|_, _, _| {}} on:middle-mouse-up={|_, _, _| {}}
            on:any-mouse-down={|_, _, _| {}}
            on:mouse-down-out={|_, _, _| {}} on:mouse-up-out={|_, _, _| {}}
            on:hover={|_: &bool, _, _| {}} on:scroll-wheel={|_, _, _| {}}
            on:drop={|_: &usize, _, _| {}}
            on:drag={(4usize, drag_preview)}
            tooltip={|_, cx| cx.new(|_| DragPreview).into()}/>
    };
}

#[test]
fn disabled_buttons_skip_new_event_builders_but_keep_tooltips() {
    use rake::gpui::AppContext;

    let seen = std::cell::RefCell::new(Vec::new());

    let _ = rake::view! {
        <button disabled={true}
            on:drag={ { seen.borrow_mut().push("drag"); (4usize, drag_preview) } }
            on:hover={ { seen.borrow_mut().push("hover"); |_, _, _| {} } }
            on:scroll-wheel={ { seen.borrow_mut().push("scroll"); |_, _, _| {} } }
            tooltip={ { seen.borrow_mut().push("tooltip"); |_, cx| cx.new(|_| DragPreview).into() } }>
            "Disabled"
        </button>
    };

    assert_eq!(*seen.borrow(), ["tooltip"]);
}

#[test]
fn semantic_containers_rules_and_svg_expose_their_documented_semantics() {
    let containers = [
        (rake::view! { <nav/> }, accesskit::Role::Navigation),
        (rake::view! { <header/> }, accesskit::Role::Banner),
        (rake::view! { <footer/> }, accesskit::Role::ContentInfo),
        (rake::view! { <main/> }, accesskit::Role::Main),
        (rake::view! { <aside/> }, accesskit::Role::Complementary),
        (rake::view! { <article/> }, accesskit::Role::Article),
    ];

    for (container, role) in containers {
        assert_eq!(container.a11y_role(), Some(role));
    }

    let decorative = rake::view! { <svg path="icons/search.svg" class="size-4"/> };

    let labelled = rake::view! {
        <svg path="icons/search.svg" aria-label="Search" aria-description="Find text"
            class="size-4" style={|s| s.text_color(rgb(0xffffff))}/>
    };

    let mut info = accesskit::Node::new(labelled.a11y_role().unwrap());

    labelled.write_a11y_info(&mut info);

    assert_eq!(decorative.a11y_role(), None);
    assert_eq!(Element::id(&labelled), None);
    assert_eq!(labelled.a11y_role(), Some(accesskit::Role::Image));
    assert_eq!(info.label(), Some("Search"));
    assert_eq!(info.description(), Some("Find text"));
    let mut rule = rake::view! { <hr/> };
    let mut expected = div().w_full().h(px(1.));

    assert_eq!(rule.a11y_role(), None);
    assert_eq!(
        format!("{:?}", rule.style()),
        format!("{:?}", expected.style())
    );
    let _ = rake::view! {
        <div>for path in ["a.svg", "b.svg"] { <svg path={path}/> }</div>
    };
}

#[test]
fn bare_boolean_attributes_and_builtin_shorthand_keep_their_types() {
    let disabled = true;
    let button = rake::view! { <button {disabled} aria-expanded>"Unavailable"</button> };
    let mut info = accesskit::Node::new(button.a11y_role().unwrap());

    button.write_a11y_info(&mut info);

    assert!(info.is_disabled());
    assert_eq!(info.is_expanded(), Some(true));
    let mut paragraph = rake::view! { <p wrap ellipsis>"Text"</p> };

    assert_eq!(
        paragraph.style().text.white_space,
        Some(rake::gpui::WhiteSpace::Normal)
    );
    assert!(paragraph.style().text.text_overflow.is_some());
    let _ = rake::view! { <section tab-navigation><button disabled/></section> };
}

#[test]
fn match_children_evaluate_only_the_selected_guarded_arm_and_keep_loop_ids() {
    let visits = std::cell::RefCell::new(Vec::new());
    let values = [Some(2usize), None];

    let _ = rake::view! {
        <div>
            for (index, value) in values.iter().enumerate() {
                match value {
                    Some(n) if *n > 1 => {
                        <span id={("value", index)}>{ { visits.borrow_mut().push(*n); "Selected" } }</span>
                    },
                    Some(_) => { <span id={("other", index)}>{unreachable_text_for_match()}</span> }
                    None => {
                        <span id={("empty", index)}>{ { visits.borrow_mut().push(0); "None" } }</span>
                    }
                }
            }
        </div>
    };

    assert_eq!(*visits.borrow(), [2, 0]);
}

fn unreachable_text_for_match() -> &'static str {
    panic!("untaken match arm")
}
