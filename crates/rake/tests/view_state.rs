//! Covers: retained entity identity, focus, selection, data, and scroll across reorders.
//! Real: GPUI entities, window layout, element state, and macro-generated trees.
//! Doubles: GPUI TestPlatform (scheduler, text system, window/GPU backend).
//! Requires: view-test-support; no display or GPU.

#![cfg(not(target_family = "wasm"))]

use rake::gpui::{self, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;

struct Draft {
    text: String,
    selection: Range<usize>,
    focus: gpui::FocusHandle,
    scroll: gpui::ScrollHandle,
}

impl Render for Draft {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <div focus={&self.focus} scroll={&self.scroll}
                style={|s| s.h(gpui::px(64.)).w(gpui::px(200.)).overflow_y_scroll()}>
                <p>{self.text.clone()}</p>
                <div style={|s| s.min_h(gpui::px(500.))}/>
            </div>
        }
    }
}

struct Row {
    id: usize,
    editor: gpui::Entity<Draft>,
}

struct RetainedHost {
    rows: Vec<Row>,
}

impl Render for RetainedHost {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <div>
                for row in &self.rows {
                    <div id={("row", row.id)}>
                        <view entity={&row.editor}/>
                    </div>
                }
            </div>
        }
    }
}

fn draw<V: Render>(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<V>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .expect("draw frame");
}

#[test]
fn entity_data_selection_focus_and_scroll_survive_parent_renders_and_reorders() {
    let mut cx = gpui::TestAppContext::single();
    let constructions = Rc::new(Cell::new(0));

    let window = cx.add_window(|_, cx| RetainedHost {
        rows: (0..2)
            .map(|id| Row {
                id,
                editor: cx.new(|cx| {
                    constructions.set(constructions.get() + 1);

                    Draft {
                        text: format!("Draft {id}"),
                        selection: 0..0,
                        focus: cx.focus_handle(),
                        scroll: gpui::ScrollHandle::new(),
                    }
                }),
            })
            .collect(),
    });

    draw(&mut cx, window);
    let editor = window
        .read_with(&cx, |host, _| host.rows[0].editor.clone())
        .expect("stored editor");

    let entity_id = editor.entity_id();

    editor.update(&mut cx, |draft, cx| {
        draft.text = "Edited draft".into();
        draft.selection = 2..7;
        draft
            .scroll
            .set_offset(gpui::point(gpui::px(0.), gpui::px(-40.)));
        cx.notify();
    });
    cx.update_window(window.into(), |_, window, cx| {
        let focus = editor.read(cx).focus.clone();

        window.focus(&focus, cx);
    })
    .expect("focus editor");

    for reverse in [false, true, false] {
        window
            .update(&mut cx, |host, _, cx| {
                if reverse {
                    host.rows.reverse();
                }

                cx.notify();
            })
            .expect("update parent");
        draw(&mut cx, window);

        window
            .read_with(&cx, |host, cx| {
                let retained = &host
                    .rows
                    .iter()
                    .find(|row| row.id == 0)
                    .expect("same row")
                    .editor;

                let draft = retained.read(cx);

                assert_eq!(retained.entity_id(), entity_id);
                assert_eq!(draft.text, "Edited draft");
                assert_eq!(draft.selection, 2..7);
                assert_eq!(draft.scroll.offset().y, gpui::px(-40.));
            })
            .expect("retained data");
        cx.update_window(window.into(), |_, window, cx| {
            assert!(editor.read(cx).focus.is_focused(window));
        })
        .expect("retained focus");
    }

    assert_eq!(
        constructions.get(),
        2,
        "rendering must not construct entities"
    );
}

struct KeyedHost {
    keys: Vec<usize>,
    observed: Rc<RefCell<BTreeMap<usize, usize>>>,
    states_created: Rc<Cell<usize>>,
}

impl Render for KeyedHost {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <div>
                for key in &self.keys {
                    <button id={("row", *key)} style={|s| s.size(gpui::px(80.))}>
                        {state_probe(*key, self.observed.clone(), self.states_created.clone())}
                    </button>
                }
            </div>
        }
    }
}

fn state_probe(
    key: usize,
    observed: Rc<RefCell<BTreeMap<usize, usize>>>,
    states_created: Rc<Cell<usize>>,
) -> impl IntoElement {
    gpui::canvas(
        move |_, window, _| {
            window.with_global_id("probe".into(), |global_id, window| {
                window.with_element_state(global_id, |state: Option<usize>, _| {
                    let state = state.unwrap_or_else(|| {
                        let next = states_created.get();

                        states_created.set(next + 1);

                        next
                    });

                    observed.borrow_mut().insert(key, state);

                    ((), state)
                });
            });
        },
        |_, _, _, _| {},
    )
    .size_full()
}

#[test]
fn keyed_element_state_and_implicit_button_focus_follow_data_keys() {
    let mut cx = gpui::TestAppContext::single();
    let observed = Rc::new(RefCell::new(BTreeMap::new()));
    let states_created = Rc::new(Cell::new(0));

    let window = cx.add_window(|_, _| KeyedHost {
        keys: vec![1, 2],
        observed: observed.clone(),
        states_created: states_created.clone(),
    });

    draw(&mut cx, window);
    let first = observed.borrow().clone();

    let focus = cx
        .update_window(window.into(), |_, window, cx| {
            window.focus_next(cx);

            window.focused(cx).expect("first button has focus")
        })
        .expect("focus first row");

    window
        .update(&mut cx, |host, _, cx| {
            host.keys.reverse();
            host.keys.insert(0, 3);
            cx.notify();
        })
        .expect("reorder rows and insert a new row");
    draw(&mut cx, window);

    let next = observed.borrow();

    assert_eq!(first.len(), 2);
    assert_eq!(next.len(), 3);
    assert_eq!(next[&1], first[&1]);
    assert_eq!(next[&2], first[&2]);
    assert_eq!(
        states_created.get(),
        3,
        "existing rows must reuse element state"
    );
    cx.update_window(window.into(), |_, window, _| {
        assert!(
            focus.is_focused(window),
            "focus must stay with the same data key"
        );
    })
    .expect("focus survives reorder");
}

#[cfg(debug_assertions)]
#[test]
fn mounting_the_same_entity_twice_in_one_scope_is_diagnosed() {
    struct DuplicateHost {
        editor: gpui::Entity<Draft>,
        detected: Rc<Cell<bool>>,
    }

    impl Render for DuplicateHost {
        fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
            let duplicate = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                rake::view! {
                    <div><view entity={&self.editor}/><view entity={&self.editor}/></div>
                }
            }));

            self.detected.set(duplicate.is_err());

            gpui::div()
        }
    }

    let mut cx = gpui::TestAppContext::single();
    let detected = Rc::new(Cell::new(false));

    let window = cx.add_window(|_, cx| DuplicateHost {
        editor: cx.new(|cx| Draft {
            text: "Draft".into(),
            selection: 0..0,
            focus: cx.focus_handle(),
            scroll: gpui::ScrollHandle::new(),
        }),
        detected: detected.clone(),
    });

    draw(&mut cx, window);

    assert!(detected.get());
}
