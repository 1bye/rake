//! Covers: public view expansion, real GPUI styles/semantics, and control-flow evaluation.
//! Real: macro expansion and GPUI element construction.
//! Doubles: none.
//! Requires: experimental-view; no display/GPU execution.

#![cfg(feature = "experimental-view")]

use std::cell::RefCell;

use rake::gpui::{Element, Styled, accesskit, div, px, rgb};

#[test]
fn conditional_classes_evaluate_once_between_their_authored_attributes() {
    let calls = RefCell::new(Vec::new());

    let _ = rake::view! {
        <div
            style={|s| { calls.borrow_mut().push(0); s }}
            class:font-bold={{ calls.borrow_mut().push(1); true }}
            aria-label={{ calls.borrow_mut().push(2); "Label" }}
            class:shrink-0={{ calls.borrow_mut().push(3); false }}
            aria-description={{ calls.borrow_mut().push(4); "Description" }}
        />
    };

    assert_eq!(*calls.borrow(), [0, 1, 2, 3, 4]);
}

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
    let _ = rake::view! { <div class="p-13 gap-1.5 gap-x-2.5 gap-y-[4px] w-[320px] h-[0.75rem] size-[50%] w-1/2 h-2/3 basis-1/3 -mt-1 -mx-[4px] -top-2 right-1 bottom-[2px] left-[10%] inset-0 text-[11px] text-[0.75rem] text-[#abc] bg-[#123456] border-[#12345678] leading-[20px] leading-none opacity-35 opacity-[0.35] font-[JetBrains_Mono] rounded-[4px] border-[2px] border-t-[2px] border-b-[1px] border-l-[3px] border-r-[0px] min-w-[12px] max-w-[50%] min-h-[1rem] max-h-[240px]"/> };
    let _ = rake::view! { <div class:w-[320px]={true} class:gap-1.5={true} class:-mt-1={true}/> };
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
fn arbitrary_utilities_set_real_lengths_colors_and_opacity() {
    let mut view = rake::view! {
        <div class="w-[320px] h-1/2 p-13 gap-1.5 -mt-1 -mx-[4px] top-2 opacity-[0.35] rounded-[4px] border-t-[2px] bg-[#abc]"/>
    };

    let mut expected = div()
        .flex()
        .flex_col()
        .w(px(320.))
        .h(rake::gpui::relative(0.5))
        .p(rake::gpui::rems(3.25))
        .gap(rake::gpui::rems(0.375))
        .mt(rake::gpui::rems(-0.25))
        .mx(px(-4.))
        .top(rake::gpui::rems(0.5))
        .opacity(0.35)
        .rounded(px(4.))
        .border_t(px(2.))
        .bg(rgb(0xaabbcc));

    assert_eq!(
        format!("{:?}", view.style()),
        format!("{:?}", expected.style())
    );
}

#[test]
fn disabled_classes_use_the_existing_predicate_and_keep_base_order() {
    let mut disabled = rake::view! { <button disabled class="opacity-100 disabled:opacity-35"/> };
    let mut enabled = rake::view! { <button class="opacity-100 disabled:opacity-35"/> };
    let mut overridden = rake::view! { <button disabled class="disabled:opacity-35 opacity-100"/> };

    assert_eq!(disabled.style().opacity, Some(0.35));
    assert_eq!(enabled.style().opacity, Some(1.));
    assert_eq!(overridden.style().opacity, Some(1.));
}

#[test]
fn literal_class_branches_evaluate_once_in_attribute_order_and_skip_untaken_conditions() {
    let visits = RefCell::new(Vec::new());

    let mut view = rake::view! {
        <div style={|s| { visits.borrow_mut().push(0); s }}
            class={if { visits.borrow_mut().push(1); true } {
                "p-4 font-bold hover:opacity-50"
            } else if { unreachable_child(); false } { "p-2" } else { "p-0" }}
            aria-label={{ visits.borrow_mut().push(2); "Branch" }}/>
    };

    let mut expected = div()
        .flex()
        .flex_col()
        .p_4()
        .font_weight(rake::gpui::FontWeight::BOLD);

    assert_eq!(*visits.borrow(), [0, 1, 2]);
    assert_eq!(
        format!("{:?}", view.style()),
        format!("{:?}", expected.style())
    );
}

#[test]
fn class_match_guards_and_if_let_patterns_inspect_noncopy_values_without_moving_them() {
    let owned = Some(String::from("retained"));
    let guards = std::cell::Cell::new(0);
    let scrutinees = std::cell::Cell::new(0);

    let mut view = rake::view! {
        <div class={match { scrutinees.set(scrutinees.get()+1); &owned } {
            Some(_) if { guards.set(guards.get()+1); true } => "w-1/2 hover:opacity-50",
            Option::None => "p-0",
            _ => "p-2",
        }}/>
    };

    let mut absent =
        rake::view! { <div class={if let Some(_) = Option::<String>::None { "p-4" }}/> };

    let mut expected = div().flex().flex_col().w(rake::gpui::relative(0.5));
    let mut empty = div().flex().flex_col();

    assert_eq!(guards.get(), 1);
    assert_eq!(scrutinees.get(), 1);
    assert_eq!(owned.as_deref(), Some("retained"));
    assert_eq!(
        format!("{:?}", view.style()),
        format!("{:?}", expected.style())
    );
    assert_eq!(
        format!("{:?}", absent.style()),
        format!("{:?}", empty.style())
    );
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

#[test]
fn every_supported_style_utility_compiles_inside_a_merged_hover_variant() {
    let _ = rake::view! { <div class="hover:flex hover:block hover:grid hover:hidden hover:flex-col hover:flex-row hover:flex-wrap hover:flex-1 hover:items-start hover:items-center hover:items-end hover:justify-start hover:justify-center hover:justify-end hover:justify-between hover:w-full hover:h-full hover:size-full hover:min-w-0 hover:min-h-0 hover:overflow-hidden hover:rounded-none hover:rounded-sm hover:rounded-md hover:rounded-lg hover:rounded-xl hover:rounded-full hover:border-0 hover:border-1 hover:border-2 hover:shadow-sm hover:shadow-md hover:shadow-lg hover:cursor-pointer hover:text-xs hover:text-sm hover:text-base hover:text-lg hover:text-xl hover:text-2xl hover:text-3xl hover:text-left"/> };
    let _ = rake::view! { <div class="hover:text-center hover:text-right hover:rounded hover:border hover:shrink-0 hover:grow hover:grow-0 hover:flex-none hover:relative hover:absolute hover:items-stretch hover:self-start hover:self-center hover:self-end hover:justify-around hover:truncate hover:whitespace-nowrap hover:overflow-x-hidden hover:overflow-y-hidden hover:border-t hover:border-t-0 hover:border-t-1 hover:border-t-2 hover:border-b hover:border-b-0 hover:border-b-1 hover:border-b-2 hover:border-l hover:border-l-0 hover:border-l-1 hover:border-l-2 hover:border-r hover:border-r-0 hover:border-r-1 hover:border-r-2 hover:p-0 hover:p-1 hover:p-2 hover:p-3 hover:p-4 hover:p-5 hover:p-6 hover:p-7 hover:p-8 hover:p-12"/> };
    let _ = rake::view! { <div class="hover:p-16 hover:p-20 hover:p-24 hover:p-32 hover:px-0 hover:px-1 hover:px-2 hover:px-3 hover:px-4 hover:px-5 hover:px-6 hover:px-7 hover:px-8 hover:px-12 hover:px-16 hover:px-20 hover:px-24 hover:px-32 hover:py-0 hover:py-1 hover:py-2 hover:py-3 hover:py-4 hover:py-5 hover:py-6 hover:py-7 hover:py-8 hover:py-12 hover:py-16 hover:py-20 hover:py-24 hover:py-32 hover:pt-0 hover:pt-1 hover:pt-2 hover:pt-3 hover:pt-4 hover:pt-5 hover:pt-6 hover:pt-7 hover:pt-8 hover:pt-12 hover:pt-16 hover:pt-20 hover:pt-24"/> };
    let _ = rake::view! { <div class="hover:pt-32 hover:pb-0 hover:pb-1 hover:pb-2 hover:pb-3 hover:pb-4 hover:pb-5 hover:pb-6 hover:pb-7 hover:pb-8 hover:pb-12 hover:pb-16 hover:pb-20 hover:pb-24 hover:pb-32 hover:pl-0 hover:pl-1 hover:pl-2 hover:pl-3 hover:pl-4 hover:pl-5 hover:pl-6 hover:pl-7 hover:pl-8 hover:pl-12 hover:pl-16 hover:pl-20 hover:pl-24 hover:pl-32 hover:pr-0 hover:pr-1 hover:pr-2 hover:pr-3 hover:pr-4 hover:pr-5 hover:pr-6 hover:pr-7 hover:pr-8 hover:pr-12 hover:pr-16 hover:pr-20 hover:pr-24 hover:pr-32 hover:m-0 hover:m-1"/> };
    let _ = rake::view! { <div class="hover:m-2 hover:m-3 hover:m-4 hover:m-5 hover:m-6 hover:m-7 hover:m-8 hover:m-12 hover:m-16 hover:m-20 hover:m-24 hover:m-32 hover:mx-0 hover:mx-1 hover:mx-2 hover:mx-3 hover:mx-4 hover:mx-5 hover:mx-6 hover:mx-7 hover:mx-8 hover:mx-12 hover:mx-16 hover:mx-20 hover:mx-24 hover:mx-32 hover:my-0 hover:my-1 hover:my-2 hover:my-3 hover:my-4 hover:my-5 hover:my-6 hover:my-7 hover:my-8 hover:my-12 hover:my-16 hover:my-20 hover:my-24 hover:my-32 hover:mt-0 hover:mt-1 hover:mt-2 hover:mt-3 hover:mt-4"/> };
    let _ = rake::view! { <div class="hover:mt-5 hover:mt-6 hover:mt-7 hover:mt-8 hover:mt-12 hover:mt-16 hover:mt-20 hover:mt-24 hover:mt-32 hover:mb-0 hover:mb-1 hover:mb-2 hover:mb-3 hover:mb-4 hover:mb-5 hover:mb-6 hover:mb-7 hover:mb-8 hover:mb-12 hover:mb-16 hover:mb-20 hover:mb-24 hover:mb-32 hover:ml-0 hover:ml-1 hover:ml-2 hover:ml-3 hover:ml-4 hover:ml-5 hover:ml-6 hover:ml-7 hover:ml-8 hover:ml-12 hover:ml-16 hover:ml-20 hover:ml-24 hover:ml-32 hover:mr-0 hover:mr-1 hover:mr-2 hover:mr-3 hover:mr-4 hover:mr-5 hover:mr-6 hover:mr-7"/> };
    let _ = rake::view! { <div class="hover:mr-8 hover:mr-12 hover:mr-16 hover:mr-20 hover:mr-24 hover:mr-32 hover:gap-0 hover:gap-1 hover:gap-2 hover:gap-3 hover:gap-4 hover:gap-5 hover:gap-6 hover:gap-7 hover:gap-8 hover:gap-12 hover:gap-16 hover:gap-20 hover:gap-24 hover:gap-32 hover:gap-x-0 hover:gap-x-1 hover:gap-x-2 hover:gap-x-3 hover:gap-x-4 hover:gap-x-5 hover:gap-x-6 hover:gap-x-7 hover:gap-x-8 hover:gap-x-12 hover:gap-x-16 hover:gap-x-20 hover:gap-x-24 hover:gap-x-32 hover:gap-y-0 hover:gap-y-1 hover:gap-y-2 hover:gap-y-3 hover:gap-y-4 hover:gap-y-5 hover:gap-y-6 hover:gap-y-7 hover:gap-y-8 hover:gap-y-12 hover:gap-y-16"/> };
    let _ = rake::view! { <div class="hover:gap-y-20 hover:gap-y-24 hover:gap-y-32 hover:w-0 hover:w-1 hover:w-2 hover:w-3 hover:w-4 hover:w-5 hover:w-6 hover:w-7 hover:w-8 hover:w-12 hover:w-16 hover:w-20 hover:w-24 hover:w-32 hover:h-0 hover:h-1 hover:h-2 hover:h-3 hover:h-4 hover:h-5 hover:h-6 hover:h-7 hover:h-8 hover:h-12 hover:h-16 hover:h-20 hover:h-24 hover:h-32 hover:size-0 hover:size-1 hover:size-2 hover:size-3 hover:size-4 hover:size-5 hover:size-6 hover:size-7 hover:size-8 hover:size-12 hover:size-16 hover:size-20 hover:size-24 hover:size-32"/> };
    let _ = rake::view! { <div class="hover:font-normal hover:font-medium hover:font-semibold hover:font-bold hover:grid-cols-1 hover:grid-cols-2 hover:grid-cols-3 hover:grid-cols-4 hover:grid-cols-5 hover:grid-cols-6 hover:grid-cols-7 hover:grid-cols-8 hover:grid-cols-9 hover:grid-cols-10 hover:grid-cols-11 hover:grid-cols-12 hover:p-13 hover:gap-1.5 hover:w-[320px] hover:h-[50%] hover:w-1/2 hover:basis-1/3 hover:-mt-1 hover:-top-2 hover:text-[11px] hover:text-[#abc] hover:bg-[#123456] hover:border-[#12345678] hover:leading-[20px] hover:leading-none hover:opacity-[0.35] hover:font-[JetBrains_Mono] hover:rounded-[4px] hover:border-t-[2px] hover:min-w-[12px] hover:max-h-[240px]"/> };
    let _ = rake::view! { <div class="group/card hover:opacity-50 active:border-2 focus:bg-[#fff] focus-visible:text-[#000] in-focus:opacity-100 group-hover/card:p-2 group-active/card:border-1"/> };
    let _ = rake::view! { <img src="x.png" alt="Preview" class="hover:opacity-50 focus-visible:border-2"/> };
    let _ = rake::view! { <button disabled class="disabled:opacity-35" class:disabled:font-bold={true}/> };
}

#[test]
fn theme_values_apply_to_native_gpui_refinements() {
    use rake::__private::{ThemeBackground, ThemeBorder, ThemeFont, ThemeRadius, ThemeText};
    use rake::gpui::{self, FontWeight};
    use rake::theme::TextToken;

    let mut style = gpui::StyleRefinement::default();

    style = ThemeBackground::apply(rgb(0x112233), style);
    style = ThemeBorder::apply(gpui::hsla(0., 1., 0.5, 1.), style);
    style = ThemeText::apply(rgb(0x445566), style);
    style = ThemeText::apply(
        TextToken {
            size: px(13.),
            line_height: Some(px(20.)),
            weight: Some(FontWeight::BOLD),
        },
        style,
    );
    style = ThemeFont::apply("Test Font", style);
    style = ThemeRadius::apply(gpui::rems(0.5), style);
    style = ThemeRadius::apply(px(4.), style);

    let mut expected = gpui::StyleRefinement::default()
        .bg(rgb(0x112233))
        .border_color(gpui::hsla(0., 1., 0.5, 1.))
        .text_color(rgb(0x445566))
        .text_size(px(13.))
        .line_height(px(20.))
        .font_weight(FontWeight::BOLD)
        .font_family("Test Font")
        .rounded(px(4.));

    assert_eq!(
        format!("{:?}", style.style()),
        format!("{:?}", expected.style())
    );
}

#[derive(Clone, rake::Theme)]
struct Theme {
    surface: rake::gpui::Rgba,
    ink: rake::gpui::Hsla,
    body: rake::theme::TextToken,
    ui: &'static str,
    control: rake::gpui::Pixels,
    hover: rake::gpui::Rgba,
}

rake::theme!(Theme);

// Installation is covered by the frame suite. This compile probe covers every
// theme prefix and all three generic/refinement emission paths.
#[allow(dead_code)]
fn theme_class_compile_probe() -> impl rake::gpui::IntoElement {
    rake::view! {
        <div class="bg-surface text-body border-ink font-ui rounded-control hover:bg-hover focus:text-ink">
            <p><span class="text-ink font-ui">Inline theme</span></p>
        </div>
    }
}

#[rake::style("h-[24px] px-2 rounded-sm")]
fn control();

#[rake::style("hover:bg-hover hover:opacity-50")]
fn hover_control();

#[rake::style("focus-visible:bg-hover active:opacity-50 group-active/card:text-ink")]
fn focus_control();

#[rake::style("focus-visible:opacity-50")]
fn focus_only();

#[rake::style("overflow-y-scroll")]
fn scrolling();

#[test]
fn presets_are_typed_style_functions_and_compose_with_builders() {
    use rake::gpui::{self, prelude::*};
    let mut actual = control(gpui::StyleRefinement::default());

    let mut expected = gpui::StyleRefinement::default()
        .h(px(24.))
        .px_2()
        .rounded_sm();

    assert_eq!(
        format!("{:?}", actual.style()),
        format!("{:?}", expected.style())
    );
    let _ = rake::view! { <div style={|s| control(s).w(px(10.))}/> };
    let _ = rake::view! { <div style={scrolling}/> };
    let _ = focus_only(gpui::div());
}

#[allow(dead_code)]
fn themed_presets_compile() -> impl rake::gpui::IntoElement {
    rake::view! { <div style={focus_control}><div style={hover_control}/></div> }
}

#[test]
fn numeric_border_widths_match_explicit_pixel_lengths() {
    let mut numeric =
        rake::view! { <div class="border-3 border-t-4 border-b-1.5 border-l-0.25 border-r-16"/> };

    let mut explicit = rake::view! { <div class="border-[3px] border-t-[4px] border-b-[1.5px] border-l-[0.25px] border-r-[16px]"/> };

    assert_eq!(
        format!("{:?}", numeric.style()),
        format!("{:?}", explicit.style())
    );
}

#[test]
fn bare_unit_variants_select_class_leaves_without_moving_the_scrutinee() {
    for value in [Some("owned".to_owned()), None] {
        let mut actual = rake::view! {
            <div class={match value { Some(_) => "p-1", None => "p-2" }}/>
        };

        let mut expected = div()
            .flex()
            .flex_col()
            .p(rake::gpui::rems(if value.is_some() { 0.25 } else { 0.5 }));

        assert_eq!(
            format!("{:?}", actual.style()),
            format!("{:?}", expected.style())
        );
    }
}

#[test]
fn class_let_chains_evaluate_once_and_short_circuit_left_to_right() {
    for (enabled, first, second, finish, expected_calls) in [
        (false, Some(1), Some(2), true, vec!["A"]),
        (true, None, Some(2), true, vec!["A", "let1"]),
        (true, Some(1), None, true, vec!["A", "let1", "let2"]),
        (
            true,
            Some(1),
            Some(2),
            false,
            vec!["A", "let1", "let2", "B"],
        ),
        (true, Some(1), Some(2), true, vec!["A", "let1", "let2", "B"]),
    ] {
        let calls = RefCell::new(Vec::new());

        let mut actual = rake::view! {
            <div class={if { calls.borrow_mut().push("A"); enabled }
                && let Some(_) = { calls.borrow_mut().push("let1"); &first }
                && let Some(_) = { calls.borrow_mut().push("let2"); &second }
                && { calls.borrow_mut().push("B"); finish } {
                    "p-1"
                } else {
                    "p-2"
                }}/>
        };

        let selected = enabled && first.is_some() && second.is_some() && finish;

        let mut expected =
            div()
                .flex()
                .flex_col()
                .p(rake::gpui::rems(if selected { 0.25 } else { 0.5 }));

        assert_eq!(*calls.borrow(), expected_calls);
        assert_eq!(
            format!("{:?}", actual.style()),
            format!("{:?}", expected.style())
        );
    }
}
