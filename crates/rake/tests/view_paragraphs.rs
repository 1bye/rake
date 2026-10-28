//! Covers: paragraph controls, inline text expressions, evaluation order, and migrations.
//! Real: public view macro and GPUI style construction.
//! Doubles: none.
//! Requires: experimental-view; no display or GPU.

#![cfg(feature = "experimental-view")]

use std::cell::RefCell;

use rake::gpui::{self, Element, Styled};

#[test]
fn paragraph_controls_and_selected_text_evaluate_once_in_source_order() {
    let seen = RefCell::new(Vec::new());
    let labels = ["é", "🙂", "你好"];
    let __rake_paragraph_content_1 = "Caller variable";

    let paragraph = rake::view! {
        <p id="summary"
            wrap={ { seen.borrow_mut().push(0); true } }
            max-lines={ { seen.borrow_mut().push(1); 3 } }
            ellipsis={ { seen.borrow_mut().push(2); true } }>
            { { seen.borrow_mut().push(3); "Hello " } }
            <strong style={ { seen.borrow_mut().push(4); gpui::FontWeight::SEMIBOLD.into() } }>
                <em>{ { seen.borrow_mut().push(5); String::from("world") } }</em>
            </strong>
            <br/>
            if false {
                {unreachable_text()}
            } else if true {
                {__rake_paragraph_content_1}
            }
            for (index, label) in labels.iter().enumerate() {
                if index > 0 { ", " }
                <span class="font-medium" font-family="monospace">
                    { { seen.borrow_mut().push(6 + index); label } }
                </span>
            }
        </p>
    };

    assert_eq!(*seen.borrow(), [0, 1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(Element::id(&paragraph), Some("summary".into()));
    assert_eq!(
        paragraph.a11y_role(),
        Some(gpui::accesskit::Role::Paragraph)
    );
}

#[test]
fn borrowed_and_shared_strings_are_text_without_display_or_element_coercion() {
    let owned = String::from("owned");
    let shared = gpui::SharedString::from("shared");

    let _paragraph = rake::view! {
        <p>{&owned}{shared}<span>" static"</span></p>
    };
}

#[test]
fn inline_literal_colors_and_font_families_compile_with_styled_runs() {
    let heading = rake::view! {
        <h2><span class="text-[#aabbcc] font-[JetBrains_Mono]">Typed text</span></h2>
    };

    let mut node = gpui::accesskit::Node::new(heading.a11y_role().unwrap());

    heading.write_a11y_info(&mut node);

    assert_eq!(node.label(), Some("Typed text"));
}

#[test]
fn wrapping_ellipsis_and_zero_line_limits_configure_the_real_gpui_style() {
    let mut paragraph = rake::view! {
        <p wrap={false} ellipsis={true} max-lines={0}>"A long title"</p>
    };

    let text = &paragraph.style().text;

    assert_eq!(text.white_space, Some(gpui::WhiteSpace::Nowrap));
    assert_eq!(text.line_clamp, Some(1));
    assert!(text.text_overflow.is_some());
}

#[test]
fn empty_paragraphs_construct_and_standalone_spans_keep_their_layout_contract() {
    let _empty = rake::view! { <p/> };

    let span = rake::view! {
        <span id="container"><button>"A widget outside a paragraph"</button></span>
    };

    assert_eq!(Element::id(&span), Some("container".into()));
}

fn unreachable_text() -> &'static str {
    panic!("untaken paragraph branch evaluated")
}

#[test]
fn headings_accept_composed_runs_and_paragraph_controls() {
    let name = "Ada";

    let mut heading = rake::view! {
        <h1 wrap={false} ellipsis={true} max-lines={2}>
            "Hello, "<strong>{name}</strong><br/><em>"Welcome"</em>
        </h1>
    };

    let mut node = gpui::accesskit::Node::new(heading.a11y_role().unwrap());

    heading.write_a11y_info(&mut node);

    assert_eq!(node.label(), Some("Hello, Ada\nWelcome"));
    assert_eq!(node.level(), Some(1));
    assert_eq!(heading.style().text.line_clamp, Some(2));
}

#[test]
fn paragraph_and_heading_match_arms_share_the_same_text_composition() {
    let selected = Some("Ada");

    let heading = rake::view! {
        <h2>"Hello, "
            match selected {
                Some(name) if !name.is_empty() => { <strong>{name}</strong> }
                _ => { <em>"guest"</em> }
            }
        </h2>
    };

    let mut node = gpui::accesskit::Node::new(heading.a11y_role().unwrap());

    heading.write_a11y_info(&mut node);

    assert_eq!(node.label(), Some("Hello, Ada"));
    let _ = rake::view! {
        <p>match selected { Some(name) => { {name} }, None => { "guest" } }</p>
    };
}

#[test]
fn unquoted_text_composes_with_inline_markup_expressions_and_control_flow() {
    let name = "Ada";
    let selected = Some("friend");

    let heading = rake::view! {
        <h2>
            Hello, <strong>{name}</strong>!<br/>
            Welcome
            back, {name}.
            if true { <em> Good</em> } else { Unreachable }
            match selected { Some(label) => { <span>{label}</span> } _ => { guest } }
            for word in ["one", "two"] { <span>{word}</span> }
        </h2>
    };

    let mut node = gpui::accesskit::Node::new(heading.a11y_role().unwrap());

    heading.write_a11y_info(&mut node);

    assert_eq!(
        node.label(),
        Some("Hello, Ada!\nWelcome back, Ada. Goodfriendonetwo")
    );
}

#[test]
fn unquoted_button_text_gets_an_accessible_label_and_quoted_spacing_stays_exact() {
    let button = rake::view! { <button>Save changes</button> };
    let heading = rake::view! { <h2>"  exact   spacing  "</h2> };
    let mut button_info = gpui::accesskit::Node::new(button.a11y_role().unwrap());
    let mut heading_info = gpui::accesskit::Node::new(heading.a11y_role().unwrap());

    button.write_a11y_info(&mut button_info);
    heading.write_a11y_info(&mut heading_info);

    assert_eq!(button_info.label(), Some("Save changes"));
    assert_eq!(heading_info.label(), Some("  exact   spacing  "));
}

#[test]
fn text_source_locations_survive_macro_rules_forwarding() {
    macro_rules! heading {
        ($($body:tt)*) => { rake::view! { <h2>$($body)*</h2> } };
    }

    macro_rules! literal_heading {
        ($text:literal) => {
            rake::view! { <h2>$text</h2> }
        };
    }

    let heading = heading!(Hello, world!);
    let quoted = literal_heading!("  exact   spacing  ");
    let mut heading_info = gpui::accesskit::Node::new(heading.a11y_role().unwrap());
    let mut quoted_info = gpui::accesskit::Node::new(quoted.a11y_role().unwrap());

    heading.write_a11y_info(&mut heading_info);
    quoted.write_a11y_info(&mut quoted_info);

    assert_eq!(heading_info.label(), Some("Hello, world!"));
    assert_eq!(quoted_info.label(), Some("  exact   spacing  "));
}
