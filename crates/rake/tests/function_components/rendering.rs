use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rake::gpui::{self, Context, IntoElement, Render, Window, prelude::*};
use rake::reactive::{ReactiveApp, Setup};

use super::{Probe, change, draw};

gpui::actions!(function_component_tests, [Activate]);

#[test]
fn automatic_cache_preserves_positioned_roots_and_sibling_flow() {
    #[derive(Default, PartialEq)]
    struct BoundsProbe {
        positioned: Cell<gpui::Bounds<gpui::Pixels>>,
        sibling: Cell<gpui::Bounds<gpui::Pixels>>,
    }

    #[rake::component]
    fn Positioned(probe: Rc<BoundsProbe>, cx: &mut Setup) -> impl IntoElement {
        let _state = cx.signal(0);

        rake::view! {
            <div class="absolute left-4 top-4 w-40 h-10">
                {
                    gpui::canvas(
                        move |bounds, _, _| probe.positioned.set(bounds),
                        |_, _, _, _| {}
                    )
                    .size_full()
                }
            </div>
        }
    }

    struct Parent(Rc<BoundsProbe>);

    impl Render for Parent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let probe = self.0.clone();

            gpui::div()
                .relative()
                .flex()
                .w(gpui::px(400.))
                .h(gpui::px(200.))
                .child(
                    Positioned::builder()
                        .id("positioned")
                        .probe(probe.clone())
                        .build(),
                )
                .child(
                    gpui::canvas(
                        move |bounds, _, _| probe.sibling.set(bounds),
                        |_, _, _, _| {},
                    )
                    .w(gpui::px(30.))
                    .h(gpui::px(40.)),
                )
        }
    }

    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(BoundsProbe::default());
    let window = cx.add_window(|_, _| Parent(probe.clone()));

    for _ in 0..3 {
        change(&mut cx, window, |_| {});
        assert_eq!(
            probe.positioned.get().origin,
            gpui::point(gpui::px(16.), gpui::px(16.))
        );
        assert_eq!(
            probe.positioned.get().size,
            gpui::size(gpui::px(160.), gpui::px(40.))
        );
        assert_eq!(
            probe.sibling.get().origin,
            gpui::point(gpui::px(0.), gpui::px(0.))
        );
    }
}

#[test]
fn cached_siblings_skip_render_when_one_signal_changes() {
    #[rake::component(cached = "w-40 h-10")]
    fn Leaf(probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
        let value = cx.signal(0);

        probe.setter.set(Some(value));

        {
            probe.value.set(value.get(cx));
            probe.renders.set(probe.renders.get() + 1);

            gpui::div().w(gpui::px(160.)).h(gpui::px(40.)).child(value)
        }
    }

    struct Parent([Rc<Probe>; 2]);

    impl Render for Parent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div().children(
                self.0
                    .iter()
                    .enumerate()
                    .map(|(id, probe)| Leaf::builder().id(id).probe(probe.clone()).build()),
            )
        }
    }
    let mut cx = gpui::TestAppContext::single();
    let probes = [Rc::new(Probe::default()), Rc::new(Probe::default())];
    let window = cx.add_window(|_, _| Parent(probes.clone()));

    draw(&mut cx, window);
    let before = [probes[0].renders.get(), probes[1].renders.get()];

    for value in 1..5 {
        cx.update(|cx| probes[0].setter.get().unwrap().set(cx, value));
        draw(&mut cx, window);
        assert_eq!(probes[0].value.get(), value);
        assert_eq!(probes[0].renders.get(), before[0] + value as usize);
        assert_eq!(probes[1].renders.get(), before[1]);
    }
}

#[test]
fn state_changes_reflow_siblings_and_run_effects_after_paint() {
    #[derive(Default)]
    struct LayoutProbe {
        state: Probe,
        painted: Cell<bool>,
        events: RefCell<Vec<i32>>,
        cleanups: Cell<usize>,
        sibling_x: Cell<f32>,
    }

    #[rake::component]
    fn Resizable(probe: Rc<LayoutProbe>, cx: &mut Setup) -> impl IntoElement {
        let width = cx.signal(100);

        probe.state.setter.set(Some(width));
        let effect_probe = probe.clone();

        cx.effect(move |_, cx| {
            let width = width.get(cx);

            assert!(effect_probe.painted.get());
            effect_probe.events.borrow_mut().push(width);
            let cleanup = effect_probe.clone();

            cx.on_cleanup(move |_| cleanup.cleanups.set(cleanup.cleanups.get() + 1));
        });

        {
            let width = width.get(cx);

            probe.painted.set(false);

            gpui::canvas(|_, _, _| (), move |_, _, _, _| probe.painted.set(true))
                .w(gpui::px(width as f32))
                .h(gpui::px(40.))
                .flex_shrink_0()
        }
    }

    struct Parent(Rc<LayoutProbe>);

    impl Render for Parent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let probe = self.0.clone();

            gpui::div()
                .flex()
                .child(Resizable {
                    id: "size".into(),
                    probe: probe.clone(),
                })
                .child(
                    gpui::canvas(
                        move |bounds, _, _| probe.sibling_x.set(bounds.origin.x.into()),
                        |_, _, _, _| {},
                    )
                    .w(gpui::px(30.))
                    .h(gpui::px(40.))
                    .flex_shrink_0(),
                )
        }
    }

    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(LayoutProbe::default());
    let window = cx.add_window(|_, _| Parent(probe.clone()));

    draw(&mut cx, window);
    assert_eq!(probe.sibling_x.get(), 100.);
    cx.update(|cx| probe.state.setter.get().unwrap().set(cx, 200));
    draw(&mut cx, window);
    assert_eq!(probe.sibling_x.get(), 200.);
    assert_eq!(*probe.events.borrow(), [100, 200]);
    assert_eq!(probe.cleanups.get(), 1);
}

#[test]
fn virtualized_function_rows_release_state_when_scrolled_out() {
    #[rake::component]
    fn Row(probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
        let value = cx.signal(0);

        probe.setter.set(Some(value));

        {
            probe.value.set(value.get(cx));

            gpui::div().h(gpui::px(32.)).child(value)
        }
    }

    struct List {
        scroll: gpui::UniformListScrollHandle,
        probes: Rc<RefCell<std::collections::BTreeMap<usize, Rc<Probe>>>>,
    }

    impl Render for List {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            gpui::uniform_list(
                "rows",
                10_000,
                cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                    range
                        .map(|id| Row {
                            id: id.into(),
                            probe: this.probes.borrow_mut().entry(id).or_default().clone(),
                        })
                        .collect()
                }),
            )
            .track_scroll(&self.scroll)
            .w(gpui::px(200.))
            .h(gpui::px(160.))
        }
    }

    let mut cx = gpui::TestAppContext::single();
    let probes = Rc::new(RefCell::new(std::collections::BTreeMap::new()));

    let window = cx.add_window(|_, _| List {
        scroll: gpui::UniformListScrollHandle::new(),
        probes: probes.clone(),
    });

    draw(&mut cx, window);
    let first = probes.borrow()[&1].clone();
    let stale = first.setter.get().unwrap();

    cx.update(|cx| stale.set(cx, 9));
    draw(&mut cx, window);
    assert_eq!(first.value.get(), 9);
    change(&mut cx, window, |list| {
        list.scroll.scroll_to_item(5000, gpui::ScrollStrategy::Top)
    });
    draw(&mut cx, window);
    assert!(
        cx.update(|cx| stale.update(cx, |value| *value = 1))
            .is_none()
    );
    assert!(probes.borrow().contains_key(&5000));
    assert!(probes.borrow().len() < 100);
    change(&mut cx, window, |list| {
        list.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top)
    });
    assert_eq!(first.value.get(), 0);
}

#[test]
fn direct_reactive_text_and_attributes_follow_signal_updates() {
    use std::fmt::{self, Display, Formatter};

    struct Label(Rc<Cell<usize>>, usize);

    impl Display for Label {
        fn fmt(&self, output: &mut Formatter<'_>) -> fmt::Result {
            self.0.set(self.1);

            write!(output, "{}", self.1)
        }
    }

    #[rake::component]
    fn ReactiveText(
        probe: Rc<Probe>,
        rendered: Rc<Cell<usize>>,
        app: &mut Setup,
    ) -> impl IntoElement {
        let selected = app.signal(0);

        probe.setter.set(Some(selected));
        let label = app.memo(move |cx| selected.get(cx).to_string());
        let heading = app.signal(Label(rendered.clone(), 7));
        let active = app.memo(move |cx| selected.get(cx) > 0);

        {
            rake::view! {
                <section>
                    <h2 aria-expanded={active} aria-label={label}>
                        <strong>{heading}</strong>
                        " / "
                        {label}
                    </h2>
                    <p wrap={active} class:font-bold={active}>
                        if active.get(app) {
                            <em>{selected}</em>
                        }
                        {label}
                    </p>
                </section>
            }
        }
    }

    struct Parent(Rc<Probe>, Rc<Cell<usize>>, Signal<&'static str>);

    impl Render for Parent {
        fn render(&mut self, _: &mut Window, app: &mut Context<Self>) -> impl IntoElement {
            let leading = self.2;

            rake::view! {
                app =>
                <div>
                    <p>{leading}</p>
                    <ReactiveText id="text" probe={self.0.clone()} rendered={self.1.clone()} />
                </div>
            }
        }
    }

    use rake::reactive::Signal;

    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(Probe::default());
    let rendered = Rc::new(Cell::new(0));
    let leading = cx.update(|cx| Signal::new(cx, "Plain view"));
    let window = cx.add_window(|_, _| Parent(probe.clone(), rendered.clone(), leading));

    draw(&mut cx, window);
    assert_eq!(rendered.replace(0), 7);

    cx.update(|cx| probe.setter.get().unwrap().set(cx, 1));
    draw(&mut cx, window);
    assert_eq!(rendered.get(), 7);
}

#[test]
fn forwarded_events_respect_reactive_button_disabled_state() {
    use rake::reactive::{Attrs, Signal};

    #[rake::component]
    fn Forwarded(blocked: Signal<bool>, #[rest] rest: Attrs) -> impl IntoElement {
        rake::view! {
            <button id="target" class="w-40 h-10" disabled={blocked}>"Target"</button>
        }
    }

    struct Preview;

    impl Render for Preview {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div()
        }
    }

    struct Parent(Signal<bool>, Rc<Cell<usize>>);

    impl Render for Parent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.1.clone();

            rake::view! {
                <Forwarded
                    blocked={self.0}
                    aria-label="Target"
                    aria-description="An interactive target"
                    aria-expanded
                    aria-keyshortcuts="Enter"
                    on:click={move |_| clicks.set(clicks.get() + 1)}
                    on:key-down={|_, _| {}}
                    on:key-up={|_| {}}
                    on:mouse-move={|_| {}}
                    on:mouse-down={|_| {}}
                    on:mouse-up={|_| {}}
                    on:right-mouse-down={|_| {}}
                    on:right-mouse-up={|_| {}}
                    on:middle-mouse-down={|_| {}}
                    on:middle-mouse-up={|_| {}}
                    on:any-mouse-down={|_| {}}
                    on:mouse-down-out={|_| {}}
                    on:mouse-up-out={|_| {}}
                    on:hover={|_| {}}
                    on:scroll-wheel={|_| {}}
                    on:drop={|_: &String, _| {}}
                    on:drag={
                        (String::from("payload"), |_, _, _, cx: &mut gpui::App|
                            cx.new(|_| Preview))
                    }
                    on:Activate={|_| {}}
                />
            }
        }
    }

    let mut cx = gpui::TestAppContext::single();
    let blocked = cx.update(|cx| Signal::new(cx, false));
    let clicks = Rc::new(Cell::new(0));
    let window = cx.add_window(|_, _| Parent(blocked, clicks.clone()));
    let position = gpui::point(gpui::px(20.), gpui::px(20.));

    for (value, expected) in [(false, 1), (true, 1), (false, 2)] {
        cx.update(|cx| blocked.set(cx, value));
        draw(&mut cx, window);
        let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);

        visual.simulate_mouse_move(position, None, Default::default());
        visual.simulate_click(position, Default::default());
        assert_eq!(clicks.get(), expected);
    }
}

#[test]
fn flex_cache_replays_only_when_the_parent_supplies_a_definite_box() {
    #[derive(Default, PartialEq)]
    struct Layout {
        mounts: Cell<usize>,
        renders: Cell<usize>,
        size: Cell<gpui::Size<gpui::Pixels>>,
    }

    #[rake::component]
    fn Stretch(probe: Rc<Layout>, cx: &mut Setup) -> impl IntoElement {
        let _state = cx.signal(0);

        probe.mounts.set(probe.mounts.get() + 1);

        rake::view! {
            <div class="flex-1 min-w-0 min-h-0">
                {
                    {
                        probe.renders.set(probe.renders.get() + 1);
                        gpui::canvas(
                            move |bounds, _, _| probe.size.set(bounds.size),
                            |_, _, _, _| {}
                        )
                        .size_full()
                    }
                }
            </div>
        }
    }

    struct Parent {
        constrained: bool,
        probe: Rc<Layout>,
    }

    impl Render for Parent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            if self.constrained {
                rake::view! {
                    <div id="parent" class="w-80 h-40 flex-row items-stretch">
                        <Stretch id="child" probe={self.probe.clone()} />
                    </div>
                }
                .into_any_element()
            } else {
                rake::view! {
                    <div id="parent" class="items-start">
                        <Stretch id="child" probe={self.probe.clone()} />
                    </div>
                }
                .into_any_element()
            }
        }
    }

    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(Layout::default());

    let window = cx.add_window(|_, _| Parent {
        constrained: true,
        probe: probe.clone(),
    });

    draw(&mut cx, window);
    assert_eq!(probe.size.get(), gpui::size(gpui::px(320.), gpui::px(160.)));

    let renders = probe.renders.get();

    change(&mut cx, window, |_| {});
    assert_eq!(probe.renders.get(), renders);

    change(&mut cx, window, |parent| parent.constrained = false);
    assert!(probe.renders.get() > renders);
    assert_eq!(probe.mounts.get(), 1);
}
