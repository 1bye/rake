//! Covers: bounded virtual rendering and retained scroll across rerenders/unmounts.
//! Real: GPUI uniform_list, retained scroll handles, layout, and view! row construction.
//! Doubles: GPUI TestPlatform's window/text/GPU backend.
//! Requires: native view-test-support; no display or GPU.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use rake::gpui::{self, prelude::*};
use rake::overlay::{Dialog, OverlayHost, OverlayHostState, OverlayState};

use super::draw;

struct Host {
    overlays: OverlayHostState,
    dialog: OverlayState,
    scroll: gpui::UniformListScrollHandle,
    rendered: Rc<RefCell<BTreeSet<usize>>>,
    shown: bool,
}

impl Render for Host {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let content = rake::view! {
            <section>
                if self.shown {
                    <section style={|s| s.w(gpui::px(240.)).h(gpui::px(160.)).overflow_hidden()}>
                        {gpui::uniform_list("large", 10_000, cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                            range.map(|index| {
                                this.rendered.borrow_mut().insert(index);

                                rake::view! {
                                    <div id={("record", index)} style={|s| s.h(gpui::px(32.))}>
                                        {format!("Record {index}")}
                                    </div>
                                }
                            }).collect()
                        })).track_scroll(&self.scroll).h(gpui::px(160.))}
                    </section>
                }
            </section>
        }.into_any_element();

        rake::view! {
            <OverlayHost state={self.overlays.clone()} body={gpui::Empty.into_any_element()}>
                <Dialog state={self.dialog.clone()} title={"Virtual records".into()} footer={None}>
                    {content}
                </Dialog>
            </OverlayHost>
        }
    }
}

#[test]
fn virtual_rows_are_bounded_and_scroll_survives_hiding_the_list() {
    let mut cx = gpui::TestAppContext::single();
    let rendered = Rc::new(RefCell::new(BTreeSet::new()));

    let window = cx.add_window(|_, cx| {
        let overlays = OverlayHostState::new(cx);

        Host {
            dialog: overlays.dialog(cx),
            overlays,
            scroll: gpui::UniformListScrollHandle::new(),
            rendered: rendered.clone(),
            shown: true,
        }
    });

    window
        .update(&mut cx, |host, window, cx| {
            host.dialog.open(window, cx);
        })
        .unwrap();

    draw(&mut cx, window);

    assert!(!rendered.borrow().is_empty());
    assert!(
        rendered.borrow().len() < 100,
        "viewport construction must not scale with all 10,000 rows"
    );
    rendered.borrow_mut().clear();
    window
        .update(&mut cx, |host, _, cx| {
            host.scroll
                .scroll_to_item(5000, gpui::ScrollStrategy::Nearest);
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);

    assert!(rendered.borrow().contains(&5000));
    assert!(rendered.borrow().len() < 100);
    window
        .update(&mut cx, |host, _, cx| {
            host.shown = false;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    rendered.borrow_mut().clear();
    window
        .update(&mut cx, |host, _, cx| {
            host.shown = true;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);

    assert!(
        rendered.borrow().contains(&5000),
        "the owner retains the scroll handle while the list is absent"
    );
    assert!(rendered.borrow().len() < 100);
}
