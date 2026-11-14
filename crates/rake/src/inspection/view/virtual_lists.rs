//! Covers: inspection of virtual-list rows laid out during prepaint.
//! Real: GPUI list layout, inspection capture, view metadata and stable identities.
//! Doubles: GPUI TestPlatform window/text/GPU backend.
//! Requires: devtools + view-test-support; no display or GPU.

use super::*;
use gpui::{ListAlignment, ListState, TestAppContext, prelude::*, px};

struct Host {
    store: Store,
    list: ListState,
    label: String,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let label = self.label.clone();

        let list = gpui::list(self.list.clone(), move |index, _, _| {
            crate::view! {
                <div id={format!("row-{index}")} style={|s| s.h(px(28.))}>
                    {format!("{label} {index}")}
                </div>
            }
            .into_any_element()
        })
        .size_full();

        inspect(gpui::div().size_full().child(list)).root(self.store.clone(), 0, WindowId(1))
    }
}

#[test]
fn virtual_list_rows_keep_inspection_metadata_when_laid_out_in_prepaint() {
    let mut cx = TestAppContext::single();
    let store = Store::default();

    store.attach();
    let window = cx.add_window(|_, _| Host {
        store: store.clone(),
        list: ListState::new(100, ListAlignment::Top, px(0.)),
        label: "Before".into(),
    });

    let draw = |cx: &mut TestAppContext| {
        cx.run_until_parked();
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .expect("draw virtual list");
    };

    draw(&mut cx);
    let first = store.snapshot();

    let row = first
        .elements
        .iter()
        .find(|e| e.semantic_id.as_deref() == Some("row-0"))
        .expect("virtual row is inspectable");

    assert!(row.view.is_some(), "authored metadata must be retained");
    let stable = row
        .stable_id
        .clone()
        .expect("virtual row has stable identity");

    window
        .update(&mut cx, |host, _, cx| {
            host.label = "After".into();
            host.list.reset(100);
            cx.notify();
        })
        .expect("change virtual rows");
    draw(&mut cx);
    let next = store.snapshot();

    let row = next
        .elements
        .iter()
        .find(|e| e.semantic_id.as_deref() == Some("row-0"))
        .expect("updated row is inspectable");

    assert_eq!(row.stable_id.as_ref(), Some(&stable));
    assert!(row.view.is_some());
    assert!(
        next.elements
            .iter()
            .any(|e| e.text.as_deref() == Some("After 0"))
    );
}
