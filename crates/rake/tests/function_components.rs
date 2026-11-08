#![cfg(not(target_family = "wasm"))]
#![doc = "Exercises real GPUI component identity, hooks, and cleanup using TestAppContext; no display or GPU required."]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rake::component::{ComponentError, Scope, State};
use rake::gpui::{self, Context, IntoElement, Render, SharedString, Window, prelude::*};

#[path = "function_components/hooks.rs"]
mod hooks;

#[derive(Default)]
struct Probe {
    value: Cell<i32>,
    setter: Cell<Option<State<i32>>>,
}

#[rake::component]
fn Counter(cx: &mut Scope, label: SharedString, start: i32, probe: Rc<Probe>) -> impl IntoElement {
    let (count, setter) = cx.use_state(|| start);
    probe.value.set(count);
    probe.setter.replace(Some(setter));

    rake::view! {
        <button
            id="increment"
            class="h-10 w-40"
            aria-label={format!("{label}: {count}")}
            on:click={
                move |_, _, cx| {
                    let _ = setter.update(cx, |value| *value += 1);
                }
            }
        >
            {format!("{label}: {count}")}
        </button>
    }
}

struct Host {
    keys: Vec<usize>,
    probes: Vec<Rc<Probe>>,
    renders: Rc<Cell<usize>>,
    start: i32,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);

        rake::view! {
            <div>
                for key in &self.keys {
                    <Counter
                        id={("counter", *key)}
                        label="Clicks"
                        start={self.start}
                        probe={self.probes[*key].clone()}
                    />
                }
            </div>
        }
    }
}

fn draw<V: Render>(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<V>) {
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    cx.run_until_parked();
}

fn change<V: Render>(
    cx: &mut gpui::TestAppContext,
    window: gpui::WindowHandle<V>,
    update: impl FnOnce(&mut V),
) {
    window
        .update(cx, |view, _, cx| {
            update(view);
            cx.notify();
        })
        .unwrap();
    draw(cx, window);
}

fn fixture() -> (
    gpui::TestAppContext,
    gpui::WindowHandle<Host>,
    Vec<Rc<Probe>>,
) {
    let mut cx = gpui::TestAppContext::single();
    let probes: Vec<_> = (0..3).map(|_| Rc::new(Probe::default())).collect();
    let window = cx.add_window(|_, _| Host {
        keys: vec![0, 1],
        probes: probes.clone(),
        renders: Rc::new(Cell::new(0)),
        start: 0,
    });
    draw(&mut cx, window);

    (cx, window, probes)
}

#[test]
fn siblings_keep_independent_state_across_parent_renders_and_keyed_changes() {
    let (mut cx, window, probes) = fixture();
    let first = probes[0].setter.get().unwrap();
    cx.update(|cx| first.set(cx, 7).unwrap());
    draw(&mut cx, window);
    assert_eq!(probes[0].value.get(), 7);
    assert_eq!(probes[1].value.get(), 0);

    change(&mut cx, window, |host| {
        host.keys = vec![2, 1, 0];
        host.start = 20;
    });
    assert_eq!(probes[0].value.get(), 7);
    assert_eq!(probes[1].value.get(), 0);
    assert_eq!(probes[2].value.get(), 20);
}

#[test]
fn button_clicks_update_current_state_through_generated_listeners() {
    let (mut cx, window, probes) = fixture();

    for expected in 1..=2 {
        let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
        let position = gpui::point(gpui::px(20.), gpui::px(20.));
        visual.simulate_mouse_move(position, None, Default::default());
        visual.simulate_click(position, Default::default());
        draw(&mut cx, window);
        assert_eq!(probes[0].value.get(), expected);
        assert_eq!(probes[1].value.get(), 0);
    }
}

#[test]
fn stale_setters_do_not_keep_components_alive_or_reach_replacements() {
    let (mut cx, window, probes) = fixture();
    let old = probes[0].setter.get().unwrap();
    change(&mut cx, window, |host| host.keys = vec![1]);
    draw(&mut cx, window);
    assert_eq!(
        cx.update(|cx| old.set(cx, 99)),
        Err(ComponentError::Disposed)
    );

    change(&mut cx, window, |host| host.keys = vec![0, 1]);
    assert_eq!(probes[0].value.get(), 0);
    assert_eq!(
        cx.update(|cx| old.set(cx, 99)),
        Err(ComponentError::Disposed)
    );
}

#[test]
fn setters_are_copy_and_work_from_async_contexts() {
    let (mut cx, window, probes) = fixture();
    let setter = probes[0].setter.get().unwrap();
    let copy = setter;
    cx.update(|cx| {
        cx.spawn(async move |cx| {
            setter.set(cx, 3).unwrap();
            copy.update(cx, |value| *value += 4).unwrap();
        })
        .detach();
    });
    draw(&mut cx, window);
    assert_eq!(probes[0].value.get(), 7);
}

#[test]
fn handles_reject_another_application() {
    let (_original, _, probes) = fixture();
    let setter = probes[0].setter.get().unwrap();
    let (other, _, _) = fixture();
    assert_eq!(
        other.update(|cx| setter.set(cx, 10)),
        Err(ComponentError::Disposed)
    );
}

#[derive(gpui::IntoElement)]
struct RawCounter {
    id: &'static str,
    handles: Rc<RefCell<Vec<gpui::Entity<i32>>>>,
}

impl gpui::RenderOnce for RawCounter {
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let state = window.use_state(cx, |_, _| 0_i32);
        self.handles.borrow_mut().push(state);
        gpui::div().id(self.id)
    }
}

struct RawHost(Rc<RefCell<Vec<gpui::Entity<i32>>>>);

impl Render for RawHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div()
            .child(RawCounter {
                id: "first",
                handles: self.0.clone(),
            })
            .child(RawCounter {
                id: "second",
                handles: self.0.clone(),
            })
    }
}

#[test]
fn raw_source_location_state_collides_before_render_once_root_ids_are_in_scope() {
    let mut cx = gpui::TestAppContext::single();
    let handles = Rc::new(RefCell::new(Vec::new()));
    let window = cx.add_window(|_, _| RawHost(handles.clone()));
    draw(&mut cx, window);
    let handles = handles.borrow();
    assert_eq!(handles[0].entity_id(), handles[1].entity_id());
}

#[test]
fn gpui_uncached_parent_is_rendered_even_for_a_leaf_notification() {
    let (mut cx, window, probes) = fixture();
    let before = window.read_with(&cx, |host, _| host.renders.get()).unwrap();
    cx.update(|cx| probes[0].setter.get().unwrap().set(cx, 1).unwrap());
    draw(&mut cx, window);
    let after = window.read_with(&cx, |host, _| host.renders.get()).unwrap();
    assert!(after > before);
}

#[test]
fn macro_reports_invalid_component_signatures() {
    trybuild::TestCases::new().compile_fail("tests/ui/component/*.rs");
}

#[test]
fn props_defaults_keep_the_existing_struct_update_syntax() {
    let probe = Rc::new(Probe::default());
    let defaults = Counter {
        id: "defaults",
        label: "Default",
        start: 4,
        probe,
    };
    let component = rake::view! {
        <Counter id="override" {..defaults} />
    };
    let _ = component.into_element();
}

#[test]
fn repeated_mounts_release_component_values_and_effect_cleanups() {
    let (mut cx, window, probes) = fixture();

    for _ in 0..100 {
        let stale = probes[0].setter.get().unwrap();
        change(&mut cx, window, |host| host.keys.clear());
        draw(&mut cx, window);
        assert_eq!(
            cx.update(|cx| stale.set(cx, 1)),
            Err(ComponentError::Disposed)
        );
        change(&mut cx, window, |host| host.keys = vec![0, 1]);
    }
}

struct Tabs {
    pages: Vec<gpui::Entity<Host>>,
    selected: usize,
}

impl Render for Tabs {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().child(self.pages[self.selected].clone())
    }
}

#[test]
fn retained_tab_entities_reset_function_state_when_their_subtree_unmounts() {
    let mut cx = gpui::TestAppContext::single();
    let probes: Vec<_> = (0..2).map(|_| Rc::new(Probe::default())).collect();
    let window = cx.add_window(|_, cx| Tabs {
        pages: probes
            .iter()
            .map(|probe| {
                cx.new(|_| Host {
                    keys: vec![0],
                    probes: vec![probe.clone()],
                    renders: Rc::default(),
                    start: 0,
                })
            })
            .collect(),
        selected: 0,
    });
    draw(&mut cx, window);
    let old = probes[0].setter.get().unwrap();
    cx.update(|cx| old.set(cx, 7).unwrap());
    draw(&mut cx, window);
    assert_eq!(probes[0].value.get(), 7);
    let page = window
        .read_with(&cx, |tabs, _| tabs.pages[0].downgrade())
        .unwrap();

    change(&mut cx, window, |tabs| tabs.selected = 1);
    draw(&mut cx, window);
    assert!(page.upgrade().is_some());
    assert_eq!(
        cx.update(|cx| old.set(cx, 9)),
        Err(ComponentError::Disposed)
    );

    change(&mut cx, window, |tabs| tabs.selected = 0);
    assert_eq!(probes[0].value.get(), 0);
}

struct Parents(Vec<(usize, Rc<Probe>)>);

impl Render for Parents {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().children(self.0.iter().map(|(parent, probe)| {
            gpui::div().id(("parent", *parent)).child(Counter {
                id: "shared-child-id",
                label: "Count",
                start: 0,
                probe: probe.clone(),
            })
        }))
    }
}

#[test]
fn identical_ids_in_different_parents_have_independent_state() {
    let mut cx = gpui::TestAppContext::single();
    let probes: Vec<_> = (0..2).map(|_| Rc::new(Probe::default())).collect();
    let window =
        cx.add_window(|_, _| Parents(vec![(8, probes[0].clone()), (19, probes[1].clone())]));
    draw(&mut cx, window);
    cx.update(|cx| probes[0].setter.get().unwrap().set(cx, 4).unwrap());
    draw(&mut cx, window);
    assert_eq!(probes[0].value.get(), 4);
    assert_eq!(probes[1].value.get(), 0);
}
