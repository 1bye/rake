//! Covers: real paragraph wrapping, inline flow, hard breaks, line caps and resizing.
//! Real: macro-generated paragraphs and GPUI measurement/layout.
//! Doubles: GPUI TestPlatform's window/text/GPU backend.
//! Requires: native view-test-support; no display or GPU.

use std::cell::RefCell;
use std::rc::Rc;

use rake::gpui::{self, prelude::*};

use super::draw;

const TEXT: &str = "one two three four five six seven eight nine ten eleven twelve";

struct Host {
    width: f32,
    bounds: Rc<RefCell<Vec<gpui::Bounds<gpui::Pixels>>>>,
}

impl Render for Host {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let observed = self.bounds.clone();

        gpui::div().flex().flex_col().text_size(gpui::px(14.)).line_height(gpui::px(20.))
            .on_children_prepainted(move |bounds, _, _| { observed.replace(bounds); })
            .children([
                rake::view! {
                    <p wrap={true} style={|s| s.w(gpui::px(self.width))}>
                        "one two "<span>{&TEXT["one two ".len()..]}</span>
                    </p>
                },
                rake::view! {
                    <p style={|s| s.w(gpui::px(self.width))}>{TEXT}</p>
                },
                rake::view! {
                    <p wrap={false} ellipsis={true} style={|s| s.w(gpui::px(self.width))}>{TEXT}</p>
                },
                rake::view! {
                    <p max-lines={2} ellipsis={true} style={|s| s.w(gpui::px(self.width))}>{TEXT}</p>
                },
                rake::view! {
                    <p wrap={false} style={|s| s.w(gpui::px(self.width))}>"first"<br/>"second"</p>
                },
                rake::view! {
                    <p max-lines={0} style={|s| s.w(gpui::px(self.width))}>{TEXT}</p>
                },
                rake::view! { <p/> },
                rake::view! {
                    <p max-lines={1} ellipsis={true} style={|s| s.w(gpui::px(self.width))}>"first"<br/>"second"</p>
                },
            ])
    }
}

#[test]
fn spans_share_one_flow_and_width_changes_remeasure_paragraphs() {
    let mut cx = gpui::TestAppContext::single();
    let bounds = Rc::new(RefCell::new(Vec::new()));

    let window = cx.add_window(|_, _| Host {
        width: 80.,
        bounds: bounds.clone(),
    });

    draw(&mut cx, window);
    let narrow = bounds.borrow().clone();

    assert_eq!(narrow.len(), 8);
    assert_eq!(
        narrow[0].size.height, narrow[1].size.height,
        "spans must share the plain text flow"
    );
    assert!(narrow[0].size.height > gpui::px(40.));
    assert_eq!(narrow[2].size.height, gpui::px(20.));
    assert_eq!(narrow[3].size.height, gpui::px(40.));
    assert_eq!(
        narrow[4].size.height,
        gpui::px(40.),
        "nowrap must preserve explicit breaks"
    );
    assert_eq!(
        narrow[5].size.height,
        gpui::px(20.),
        "zero caps normalize to one line"
    );
    assert_eq!(
        narrow[7].size.height,
        gpui::px(20.),
        "caps must count explicit breaks"
    );

    window
        .update(&mut cx, |host, _, cx| {
            host.width = 600.;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    let wide = bounds.borrow();

    assert!(wide[0].size.height < narrow[0].size.height);
    assert_eq!(wide[0].size.height, wide[1].size.height);
    assert_eq!(wide[7].size.height, gpui::px(20.));
}

struct HeadingHost {
    info: Rc<RefCell<Vec<gpui::accesskit::Node>>>,
    bounds: Rc<RefCell<Vec<gpui::Bounds<gpui::Pixels>>>>,
}

impl Render for HeadingHost {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let count = 42;

        let heading = rake::view! {
            <h2 max-lines={1} style={|s| s.w(gpui::px(400.))}>
                "Count: "<strong>{count.to_string()}</strong><em>" finished"</em>
            </h2>
        };

        let explicit = rake::view! {
            <h3 aria-label="Custom name">"Visible "<strong>"text"</strong></h3>
        };

        let mut info = Vec::new();

        // TestPlatform has no active AccessKit adapter. Inspect the real nodes
        // produced during Render, alongside real frame measurement below.
        for element in [&heading, &explicit] {
            let mut node = gpui::accesskit::Node::new(element.a11y_role().expect("heading role"));

            element.write_a11y_info(&mut node);
            info.push(node);
        }

        self.info.replace(info);
        let bounds = self.bounds.clone();

        gpui::div()
            .flex()
            .flex_col()
            .line_height(gpui::px(24.))
            .on_children_prepainted(move |measured, _, _| {
                bounds.replace(measured);
            })
            .child(heading)
            .child(explicit)
    }
}

#[test]
fn headings_share_one_text_flow_and_expose_the_full_composed_name() {
    let mut cx = gpui::TestAppContext::single();
    let info = Rc::new(RefCell::new(Vec::new()));
    let bounds = Rc::new(RefCell::new(Vec::new()));

    let window = cx.add_window(|_, _| HeadingHost {
        info: info.clone(),
        bounds: bounds.clone(),
    });

    draw(&mut cx, window);
    let info = info.borrow();

    assert_eq!(info[0].role(), gpui::accesskit::Role::Heading);
    assert_eq!(info[0].level(), Some(2));
    assert_eq!(info[0].label(), Some("Count: 42 finished"));
    assert_eq!(info[1].label(), Some("Custom name"));
    assert_eq!(info[1].level(), Some(3));
    assert_eq!(bounds.borrow()[0].size.height, gpui::px(24.));
}
