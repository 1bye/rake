//! Covers: component root identity and retained children across parent rerenders/reorders.
//! Real: GPUI RenderOnce, entity rendering, focus, layout, and keyed element state.
//! Doubles: GPUI TestPlatform's window/text/GPU backend.
//! Requires: the parent view-test-support target; no display or GPU.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use rake::gpui::{self, prelude::*};

use super::{Draft, draw, state_probe};

#[derive(IntoElement)]
struct Tile {
    id: gpui::ElementId,
    children: Vec<gpui::AnyElement>,
}

impl RenderOnce for Tile {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        rake::view! {
            <button id={self.id} style={|s| s.size(gpui::px(80.)).children(self.children)}/>
        }
    }
}

struct ComponentHost {
    keys: Vec<usize>,
    editor: gpui::Entity<Draft>,
    observed: Rc<RefCell<BTreeMap<usize, usize>>>,
    states_created: Rc<Cell<usize>>,
}

impl Render for ComponentHost {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <section>
                for key in &self.keys {
                    <Tile id={("tile", *key).into()}>
                        {state_probe(*key, self.observed.clone(), self.states_created.clone())}
                    </Tile>
                }
                <Tile id={"editor".into()}>
                    <view entity={&self.editor}/>
                </Tile>
            </section>
        }
    }
}

#[test]
fn component_ids_keep_state_and_focus_while_retained_children_keep_their_owner() {
    let mut cx = gpui::TestAppContext::single();
    let observed = Rc::new(RefCell::new(BTreeMap::new()));
    let states_created = Rc::new(Cell::new(0));
    let entities_created = Cell::new(0);

    let window = cx.add_window(|_, cx| ComponentHost {
        keys: vec![1, 2],
        editor: cx.new(|cx| {
            entities_created.set(entities_created.get() + 1);

            Draft {
                text: "Owned draft".into(),
                selection: 2..5,
                focus: cx.focus_handle(),
                scroll: gpui::ScrollHandle::new(),
            }
        }),
        observed: observed.clone(),
        states_created: states_created.clone(),
    });

    draw(&mut cx, window);
    let original_states = observed.borrow().clone();

    let entity_id = window
        .read_with(&cx, |host, _| host.editor.entity_id())
        .unwrap();

    let focus = cx
        .update_window(window.into(), |_, window, cx| {
            window.focus_next(cx);

            window.focused(cx).expect("first tile focused")
        })
        .unwrap();

    window
        .update(&mut cx, |host, _, cx| {
            host.keys.reverse();
            host.keys.insert(0, 3);
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);

    assert_eq!(observed.borrow()[&1], original_states[&1]);
    assert_eq!(observed.borrow()[&2], original_states[&2]);
    assert_eq!(states_created.get(), 3);
    assert_eq!(entities_created.get(), 1);
    window
        .update(&mut cx, |host, window, cx| {
            assert!(focus.is_focused(window));
            assert_eq!(host.editor.entity_id(), entity_id);
            assert_eq!(host.editor.read(cx).text, "Owned draft");
            assert_eq!(host.editor.read(cx).selection, 2..5);
        })
        .unwrap();
}
