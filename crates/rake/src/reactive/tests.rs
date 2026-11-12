use std::cell::Cell;
use std::rc::Rc;

use crate::gpui::{self, AppContext};

use super::Signal;

#[test]
fn signals_borrow_non_clone_values_and_preserve_entity_identity() {
    struct Value(String);
    let cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        let entity = cx.new(|_| Value("before".into()));
        let signal = Signal::from_entity(cx, entity.clone());
        assert_eq!(signal.entity(cx), entity);
        assert_eq!(signal.read(cx).0, "before");
        signal.update(cx, |value| value.0 = "after".into());
        assert_eq!(entity.read(cx).0, "after");
        assert_eq!(signal.with(cx, |value| value.0.len()), 5);
    });
}

#[test]
fn stale_signals_cannot_update_reused_slots_or_another_application() {
    let cx = gpui::TestAppContext::single();
    let stale = cx.update(|cx| {
        let stale = Signal::new(cx, 1);
        stale.dispose(cx);
        let replacement = Signal::new(cx, 2);
        assert_ne!(stale.key.slot, replacement.key.slot);
        assert!(!stale.is_alive(cx));
        stale.set(cx, 99);
        assert_eq!(replacement.get(cx), 2);
        stale
    });
    let other = gpui::TestAppContext::single();
    other.update(|cx| {
        stale.set(cx, 99);
        let current = Signal::new(cx, 7);
        stale.set(cx, 99);
        assert_eq!(current.get(cx), 7);
    });
}

#[test]
fn set_if_changed_suppresses_equal_notifications() {
    let cx = gpui::TestAppContext::single();
    let notifications = Rc::new(Cell::new(0));
    let (signal, _subscription) = cx.update(|cx| {
        let signal = Signal::new(cx, 1);
        let count = notifications.clone();
        let subscription = cx.observe(&signal.entity(cx), move |_, _| count.set(count.get() + 1));
        (signal, subscription)
    });
    cx.run_until_parked();
    cx.update(|cx| signal.set_if_changed(cx, 1));
    cx.run_until_parked();
    assert_eq!(notifications.get(), 0);
    cx.update(|cx| signal.set_if_changed(cx, 2));
    cx.run_until_parked();
    assert_eq!(notifications.get(), 1);
}

#[test]
fn setup_runs_once_and_render_uses_current_props() {
    use super::Setup;
    use crate::gpui::{Context, IntoElement, Render, Window, prelude::*};

    #[derive(Default)]
    struct Probe {
        mounts: Cell<usize>,
        signal: Cell<Option<Signal<i32>>>,
        rendered: Cell<(i32, i32)>,
    }

    #[crate::component]
    fn Counter(start: i32, probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
        probe.mounts.set(probe.mounts.get() + 1);
        let value = cx.signal(start);
        probe.signal.set(Some(value));
        {
            probe.rendered.set((start, value.get(cx)));
            gpui::div().child(value.get(cx).to_string())
        }
    }

    struct Host {
        start: i32,
        probe: Rc<Probe>,
    }
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Counter::builder()
                .id("counter")
                .start(self.start)
                .probe(self.probe.clone())
                .build()
        }
    }
    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(Probe::default());
    let window = cx.add_window(|_, _| Host {
        start: 1,
        probe: probe.clone(),
    });
    let draw = |cx: &mut gpui::TestAppContext| {
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    draw(&mut cx);
    assert_eq!(probe.mounts.get(), 1);
    let value = probe.signal.get().unwrap();
    cx.update(|cx| value.set(cx, 7));
    draw(&mut cx);
    assert_eq!(probe.rendered.get(), (1, 7));
    window
        .update(&mut cx, |host, _, cx| {
            host.start = 9;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx);
    assert_eq!(probe.mounts.get(), 1);
    assert_eq!(probe.rendered.get(), (9, 7));
}

#[test]
fn memos_replace_dynamic_dependencies_and_notify_only_on_change() {
    use super::{Memo, Setup};
    use crate::gpui::{Context, IntoElement, Render, Window, prelude::*};

    #[derive(Default)]
    struct Probe {
        select: Cell<Option<Signal<bool>>>,
        left: Cell<Option<Signal<i32>>>,
        right: Cell<Option<Signal<i32>>>,
        memo: Cell<Option<Memo<i32>>>,
        chain: Cell<Option<Memo<i32>>>,
        runs: Cell<usize>,
    }
    #[crate::component]
    fn Derived(probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
        let select = cx.signal(true);
        let left = cx.signal(2);
        let right = cx.signal(5);
        probe.select.set(Some(select));
        probe.left.set(Some(left));
        probe.right.set(Some(right));
        let counter = probe.clone();
        let memo = cx.memo(move |cx| {
            counter.runs.set(counter.runs.get() + 1);
            if select.get(cx) {
                left.get(cx)
            } else {
                right.get(cx)
            }
        });
        probe.memo.set(Some(memo));
        let mut chain = memo;
        for _ in 0..50 {
            let previous = chain;
            chain = cx.memo(move |cx| previous.get(cx) + 1);
        }
        probe.chain.set(Some(chain));
        gpui::div().child(chain)
    }
    struct Host(Rc<Probe>);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Derived::builder()
                .id("derived")
                .probe(self.0.clone())
                .build()
        }
    }
    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(Probe::default());
    let window = cx.add_window(|_, _| Host(probe.clone()));
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    cx.run_until_parked();
    let memo = probe.memo.get().unwrap();
    assert_eq!(cx.update(|cx| memo.get(cx)), 2);
    cx.update(|cx| probe.right.get().unwrap().set(cx, 7));
    cx.run_until_parked();
    assert_eq!(probe.runs.get(), 1);
    cx.update(|cx| probe.select.get().unwrap().set(cx, false));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| memo.get(cx)), 7);
    assert_eq!(cx.update(|cx| probe.chain.get().unwrap().get(cx)), 57);
    let runs = probe.runs.get();
    cx.update(|cx| probe.left.get().unwrap().set(cx, 9));
    cx.run_until_parked();
    assert_eq!(probe.runs.get(), runs);
}

#[test]
fn resources_refetch_and_keep_the_latest_success_without_clone() {
    use super::{Resource, Setup};
    use crate::gpui::{Context, IntoElement, Render, Window, prelude::*};
    use crate::loading::LoadStatus;
    use futures::channel::oneshot;
    use std::cell::RefCell;

    struct Value(i32);
    #[derive(Default)]
    struct Probe {
        input: Cell<Option<Signal<i32>>>,
        resource: Cell<Option<Resource<Value, std::io::Error>>>,
        requests: RefCell<Vec<(i32, oneshot::Sender<Value>)>>,
    }
    #[crate::component]
    fn Request(probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
        let input = cx.signal(1);
        probe.input.set(Some(input));
        let requests = probe.clone();
        let resource = cx.resource(move |cx| {
            let value = input.get(cx);
            let (send, receive) = oneshot::channel();
            requests.requests.borrow_mut().push((value, send));
            async move { receive.await.map_err(std::io::Error::other) }
        });
        probe.resource.set(Some(resource));
        gpui::div().child(if resource.is_loading(cx) {
            "Loading"
        } else {
            "Ready"
        })
    }
    struct Host(Rc<Probe>, bool);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div().when(self.1, |div| {
                div.child(
                    Request::builder()
                        .id("request")
                        .probe(self.0.clone())
                        .build(),
                )
            })
        }
    }
    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(Probe::default());
    let window = cx.add_window(|_, _| Host(probe.clone(), true));
    let draw = |cx: &mut gpui::TestAppContext| {
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.run_until_parked();
    };
    draw(&mut cx);
    let resource = probe.resource.get().unwrap();
    assert!(cx.update(|cx| resource.is_loading(cx)));
    let (input, send) = probe.requests.borrow_mut().pop().unwrap();
    assert_eq!(input, 1);
    assert!(send.send(Value(10)).is_ok());
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| resource.latest(cx).unwrap().0), 10);
    cx.update(|cx| probe.input.get().unwrap().set(cx, 2));
    cx.run_until_parked();
    assert!(cx.update(|cx| matches!(resource.read(cx), LoadStatus::Loading(_))));
    assert_eq!(cx.update(|cx| resource.latest(cx).unwrap().0), 10);
    let (_, cancelled) = probe.requests.borrow_mut().pop().unwrap();
    cx.update(|cx| resource.retry(cx));
    cx.run_until_parked();
    assert!(cancelled.send(Value(99)).is_err());
    let (_, failed) = probe.requests.borrow_mut().pop().unwrap();
    drop(failed);
    cx.run_until_parked();
    assert!(cx.update(|cx| matches!(resource.read(cx), LoadStatus::Failed(_))));
    assert_eq!(cx.update(|cx| resource.latest(cx).unwrap().0), 10);
    cx.update(|cx| resource.retry(cx));
    cx.run_until_parked();
    let (_, retried) = probe.requests.borrow_mut().pop().unwrap();
    assert!(retried.send(Value(20)).is_ok());
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| resource.latest(cx).unwrap().0), 20);
    for value in 3..103 {
        cx.update(|cx| probe.input.get().unwrap().set(cx, value));
        cx.run_until_parked();
    }
    let (input, latest) = probe.requests.borrow_mut().pop().unwrap();
    assert_eq!(input, 102);
    for (_, cancelled) in probe.requests.borrow_mut().drain(..) {
        assert!(cancelled.send(Value(99)).is_err());
    }
    assert!(latest.send(Value(102)).is_ok());
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| resource.latest(cx).unwrap().0), 102);
    cx.update(|cx| resource.retry(cx));
    cx.run_until_parked();
    let (_, late) = probe.requests.borrow_mut().pop().unwrap();
    window
        .update(&mut cx, |host, _, cx| {
            host.1 = false;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx);
    draw(&mut cx);
    assert!(late.send(Value(99)).is_err());
    assert!(!cx.update(|cx| probe.input.get().unwrap().is_alive(cx)));
}

#[test]
fn effects_run_after_paint_and_clean_up_before_rerun_and_unmount() {
    use super::{ReactiveApp, Setup};
    use crate::gpui::{Context, IntoElement, Render, Window, prelude::*};
    use std::cell::RefCell;

    #[derive(Default)]
    struct Probe {
        input: Cell<Option<Signal<i32>>>,
        painted: Cell<bool>,
        events: RefCell<Vec<i32>>,
    }
    #[crate::component]
    fn Effectful(probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
        let input = cx.signal(1);
        probe.input.set(Some(input));
        let effect_probe = probe.clone();
        cx.effect(move |_, cx| {
            assert!(effect_probe.painted.get());
            let value = input.get(cx);
            effect_probe.events.borrow_mut().push(value);
            let cleanup_probe = effect_probe.clone();
            cx.on_cleanup(move |cx| {
                assert!(input.is_alive(cx));
                cleanup_probe.events.borrow_mut().push(-value);
            });
        });
        gpui::canvas(|_, _, _| (), move |_, _, _, _| probe.painted.set(true)).size(gpui::px(20.))
    }
    struct Host(Rc<Probe>, bool);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div().when(self.1, |div| {
                div.child(
                    Effectful::builder()
                        .id("effect")
                        .probe(self.0.clone())
                        .build(),
                )
            })
        }
    }
    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(Probe::default());
    let window = cx.add_window(|_, _| Host(probe.clone(), true));
    let draw = |cx: &mut gpui::TestAppContext| {
        cx.update_window(window.into(), |_, window, cx| {
            window.draw(cx).clear(cx);
            window.simulate_next_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    };
    draw(&mut cx);
    assert_eq!(*probe.events.borrow(), [1]);
    cx.update(|cx| probe.input.get().unwrap().set(cx, 2));
    draw(&mut cx);
    assert_eq!(*probe.events.borrow(), [1, -1, 2]);
    window
        .update(&mut cx, |host, _, cx| {
            host.1 = false;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx);
    draw(&mut cx);
    assert_eq!(*probe.events.borrow(), [1, -1, 2, -2]);
}

#[test]
fn tracked_props_refetch_and_children_render_with_their_owner() {
    use super::{Children, Setup};
    use crate::gpui::{Context, IntoElement, Render, Window, prelude::*};

    #[derive(Default)]
    struct Probe {
        input: Cell<i32>,
        runs: Cell<usize>,
        updates: Cell<Option<Signal<i32>>>,
        children: Cell<usize>,
        loaded: Cell<Option<i32>>,
    }

    #[crate::component]
    fn Panel(
        #[prop(track)] input: i32,
        probe: Rc<Probe>,
        children: Children,
        cx: &mut Setup,
    ) -> impl IntoElement {
        let count = cx.signal(0);
        probe.updates.set(Some(count));
        let compute = probe.clone();
        let value = cx.memo(move |cx| {
            compute.input.set(input.get(cx));
            compute.runs.set(compute.runs.get() + 1);
            input.get(cx)
        });
        let resource = cx.resource(move |cx| {
            let input = input.get(cx);
            async move { Ok::<_, std::io::Error>(input) }
        });
        {
            probe.loaded.set(resource.latest(cx).copied());
            crate::view! {
                <div>{count}{value}{children}</div>
            }
        }
    }

    struct Host {
        input: i32,
        probe: Rc<Probe>,
    }
    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let probe = self.probe.clone();
            crate::view! {
                <Panel id="panel" input={self.input} probe={self.probe.clone()}>
                    {
                        {
                            probe.children.set(probe.children.get() + 1);
                            gpui::div()
                        }
                    }
                </Panel>
            }
        }
    }
    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(Probe::default());
    let window = cx.add_window(|_, _| Host {
        input: 1,
        probe: probe.clone(),
    });
    let draw = |cx: &mut gpui::TestAppContext| {
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.run_until_parked();
    };
    draw(&mut cx);
    assert_eq!(probe.input.get(), 1);
    let before = probe.children.get();
    cx.update(|cx| probe.updates.get().unwrap().set(cx, 2));
    draw(&mut cx);
    assert!(probe.children.get() > before);
    window
        .update(&mut cx, |host, _, cx| {
            host.input = 3;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx);
    assert_eq!(probe.input.get(), 3);
    assert_eq!(probe.runs.get(), 2);
    draw(&mut cx);
    assert_eq!(probe.loaded.get(), Some(3));
}

#[test]
fn props_builders_support_generics_defaults_spreads_and_events() {
    use crate::gpui::{IntoElement, ParentElement, SharedString};

    #[crate::component]
    fn Label<T: ToString + 'static>(
        value: T,
        #[prop(default = 2)] copies: usize,
        disabled: bool,
    ) -> impl IntoElement {
        let text = if disabled {
            String::new()
        } else {
            value.to_string().repeat(copies)
        };
        gpui::div().child(text)
    }
    let label = Label::builder().value(5_u32).build();
    assert_eq!(label.copies, 2);
    assert!(!label.disabled);
    let label = Label::builder_from(label).copies(3_usize).build();
    assert_eq!(label.copies, 3);

    #[crate::component]
    fn Button(label: SharedString, #[event] click: gpui::ClickEvent) -> impl IntoElement {
        crate::view! {
            <button
                aria-label={label.clone()}
                on:click={move |event, window, cx| click.call(event, window, cx)}
            >
                {label}
            </button>
        }
    }
    let button = Button::builder()
        .label("Save")
        .on_click(|_, _, _| {})
        .build();
    assert_eq!(button.label.as_ref(), "Save");
}

#[test]
fn named_props_and_short_event_handlers_use_the_public_builder() {
    use crate::gpui::{IntoElement, SharedString};

    #[derive(crate::Props)]
    struct LabelProps {
        text: SharedString,
        disabled: bool,
    }
    #[crate::component]
    fn Label(props: LabelProps) -> impl IntoElement {
        crate::view! {
            <button
                aria-label={props.text.clone()}
                disabled={props.disabled}
                on:click={|cx| cx.refresh_windows()}
            >
                {props.text}
            </button>
        }
    }
    let label = Label::from(Label::builder().text("Named").build());
    assert_eq!(label.props.text.as_ref(), "Named");
    assert!(!label.props.disabled);
}

#[test]
fn context_globals_and_owned_tasks_follow_component_lifetimes() {
    use super::{ReactiveApp, Setup};
    use crate::gpui::{Context, IntoElement, Render, Window, prelude::*};
    use futures::channel::oneshot;
    use std::cell::RefCell;

    struct Active(Rc<Cell<usize>>);
    impl Drop for Active {
        fn drop(&mut self) {
            self.0.set(self.0.get() - 1);
        }
    }
    #[derive(Default)]
    struct Probe {
        signal: Cell<Option<Signal<i32>>>,
        value: Cell<i32>,
        active: Rc<Cell<usize>>,
        sender: RefCell<Option<oneshot::Sender<()>>>,
    }
    #[crate::component]
    fn Consumer(probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
        let shared = cx.context::<Signal<i32>>().unwrap();
        let global = cx.global_signal::<String>();
        {
            probe.value.set(shared.get(cx));
            gpui::div().child(global.get(cx))
        }
    }
    #[crate::component]
    fn Provider(probe: Rc<Probe>, cx: &mut Setup) -> impl IntoElement {
        let signal = cx.signal(3);
        probe.signal.set(Some(signal));
        cx.provide(signal);
        let (send, receive) = oneshot::channel();
        probe.sender.replace(Some(send));
        let active = probe.active.clone();
        cx.spawn(async move |cx| {
            active.set(active.get() + 1);
            let _active = Active(active);
            if receive.await.is_ok() {
                signal.set(cx, 9);
            }
        });
        Consumer::builder().id("consumer").probe(probe).build()
    }
    struct Host(Rc<Probe>, bool);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            gpui::div().when(self.1, |div| {
                div.child(
                    Provider::builder()
                        .id("provider")
                        .probe(self.0.clone())
                        .build(),
                )
            })
        }
    }
    let mut cx = gpui::TestAppContext::single();
    cx.update(|cx| {
        super::provide_global(cx, String::from("Global"));
    });
    let probe = Rc::new(Probe::default());
    let window = cx.add_window(|_, _| Host(probe.clone(), true));
    let draw = |cx: &mut gpui::TestAppContext| {
        cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.run_until_parked();
    };
    draw(&mut cx);
    assert_eq!(probe.value.get(), 3);
    assert_eq!(probe.active.get(), 1);
    cx.update(|cx| probe.signal.get().unwrap().set(cx, 5));
    draw(&mut cx);
    assert_eq!(probe.value.get(), 5);
    window
        .update(&mut cx, |host, _, cx| {
            host.1 = false;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx);
    draw(&mut cx);
    assert_eq!(probe.active.get(), 0);
    assert_eq!(Rc::strong_count(&probe), 2);
    assert!(probe.sender.borrow_mut().take().unwrap().send(()).is_err());
    assert!(!cx.update(|cx| probe.signal.get().unwrap().is_alive(cx)));
}

#[test]
fn rest_attributes_and_signal_booleans_preserve_button_behavior() {
    use super::{Attrs, Setup};
    use crate::gpui::{Context, IntoElement, Render, Window, prelude::*};

    #[crate::component]
    fn Button(#[rest] rest: Attrs) -> impl IntoElement {
        crate::view! {
            <button>"Save"</button>
        }
    }
    #[crate::component]
    fn Controls(clicks: Rc<Cell<usize>>, cx: &mut Setup) -> impl IntoElement {
        let disabled = cx.signal(true);
        crate::view! {
            <div>
                <Button
                    class="w-40 h-10"
                    aria-label="Save document"
                    disabled={true}
                    on:click={
                        move |cx| {
                            let _ = cx;
                            clicks.set(clicks.get() + 1);
                        }
                    }
                />
                <button id="signal-button" class="w-40 h-10" disabled={disabled}>
                    "Signal disabled"
                </button>
            </div>
        }
    }
    struct Host(Rc<Cell<usize>>);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Controls::builder()
                .id("controls")
                .clicks(self.0.clone())
                .build()
        }
    }
    let mut cx = gpui::TestAppContext::single();
    cx.update(rake_ui::init);
    let clicks = Rc::new(Cell::new(0));
    let window = cx.add_window(|_, _| Host(clicks.clone()));
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_click(
        gpui::point(gpui::px(20.), gpui::px(20.)),
        Default::default(),
    );
    visual.simulate_click(
        gpui::point(gpui::px(20.), gpui::px(60.)),
        Default::default(),
    );
    assert_eq!(clicks.get(), 0);
}
