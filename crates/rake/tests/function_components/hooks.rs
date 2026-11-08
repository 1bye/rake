use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rake::component::{FunctionComponent, Resource, Scope, State, StateRef};
use rake::gpui::{self, Context, IntoElement, Render, Window, prelude::*};
use rake::prelude::{
    Collector, HttpClient, HttpConfig, RouterConfig, Runtime, ServiceRegistry, Services,
};

use super::{change, draw};

struct Host<P> {
    props: P,
    render: fn(P) -> gpui::AnyElement,
    visible: bool,
    _driver: Option<gpui::Task<()>>,
}

impl<P: Clone + 'static> Render for Host<P> {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::div().when(self.visible, |div| {
            div.child((self.render)(self.props.clone()))
        })
    }
}

fn mount<P: Clone + IntoElement + 'static>(
    cx: &mut gpui::TestAppContext,
    props: P,
) -> gpui::WindowHandle<Host<P>> {
    cx.add_window(|_, _| Host {
        props,
        render: IntoElement::into_any_element,
        visible: true,
        _driver: None,
    })
}

fn refresh<P: Clone + 'static>(cx: &mut gpui::TestAppContext, window: gpui::WindowHandle<Host<P>>) {
    change(cx, window, |_| {});
}

#[derive(Default)]
struct LifecycleProbe {
    events: RefCell<Vec<String>>,
    painted: Cell<bool>,
    memo_calls: Cell<usize>,
    reference: Cell<Option<StateRef<String>>>,
    reference_value: RefCell<String>,
}

#[rake::component]
fn Lifecycle(cx: &mut Scope, dependency: i32, probe: Rc<LifecycleProbe>) -> impl IntoElement {
    probe.painted.set(false);
    let (count, _) = cx.use_state(|| 0);
    let reference = cx.use_ref(|| String::from("retained"));
    probe
        .reference_value
        .replace(reference.read(cx, String::clone).unwrap());
    probe.reference.replace(Some(reference));
    let memo = cx.use_memo(dependency, |value| {
        probe.memo_calls.set(probe.memo_calls.get() + 1);
        value * 2
    });
    let effect_probe = probe.clone();
    cx.use_effect(dependency, move |_| {
        assert!(effect_probe.painted.get());
        effect_probe
            .events
            .borrow_mut()
            .push(format!("effect:{dependency}"));
        move || {
            effect_probe
                .events
                .borrow_mut()
                .push(format!("cleanup:{dependency}"))
        }
    });

    gpui::div()
        .child(format!("{count}:{memo}"))
        .child(gpui::canvas(
            |_, _, _| (),
            move |_, _, _, _| probe.painted.set(true),
        ))
}

#[test]
fn memo_ref_and_effects_follow_dependencies_and_mount_lifetime() {
    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(LifecycleProbe::default());
    let window = mount(
        &mut cx,
        Lifecycle {
            id: "lifecycle",
            dependency: 1,
            probe: probe.clone(),
        },
    );
    draw(&mut cx, window);
    assert_eq!(*probe.events.borrow(), ["effect:1"]);
    assert_eq!(probe.memo_calls.get(), 1);
    cx.update(|cx| {
        probe
            .reference
            .get()
            .unwrap()
            .update(cx, |value| value.push_str(" data"))
            .unwrap()
    });
    refresh(&mut cx, window);
    assert_eq!(*probe.reference_value.borrow(), "retained data");
    assert_eq!(*probe.events.borrow(), ["effect:1"]);
    assert_eq!(probe.memo_calls.get(), 1);

    change(&mut cx, window, |host| host.props.dependency = 2);
    assert_eq!(
        *probe.events.borrow(),
        ["effect:1", "cleanup:1", "effect:2"]
    );
    assert_eq!(probe.memo_calls.get(), 2);

    change(&mut cx, window, |host| host.visible = false);
    draw(&mut cx, window);
    assert_eq!(
        *probe.events.borrow(),
        ["effect:1", "cleanup:1", "effect:2", "cleanup:2"]
    );
    assert!(
        cx.update(|cx| probe.reference.get().unwrap().read(cx, String::clone))
            .is_err()
    );
}

#[rake::component]
fn EffectSetter(cx: &mut Scope, value: Rc<Cell<i32>>) -> impl IntoElement {
    let (count, setter) = cx.use_state(|| 0);
    value.set(count);
    cx.use_effect((), move |cx| {
        setter.set(cx, 7).unwrap();
    });
    gpui::div()
}

#[test]
fn effects_can_set_state_after_the_render_borrow_has_ended() {
    let mut cx = gpui::TestAppContext::single();
    let value = Rc::new(Cell::new(0));
    let window = mount(
        &mut cx,
        EffectSetter {
            id: "effect",
            value: value.clone(),
        },
    );
    draw(&mut cx, window);
    draw(&mut cx, window);
    assert_eq!(value.get(), 7);
}

#[derive(Clone)]
struct Settings(i32);

impl gpui::Global for Settings {}

#[rake::component]
fn Subscriber(cx: &mut Scope, value: Rc<Cell<i32>>) -> impl IntoElement {
    value.set(cx.use_context::<Settings>().0);
    gpui::div()
}

#[test]
fn global_changes_reach_subscribed_components() {
    let mut cx = gpui::TestAppContext::single();
    cx.update(|cx| cx.set_global(Settings(1)));
    let value = Rc::new(Cell::new(0));
    let window = mount(
        &mut cx,
        Subscriber {
            id: "subscriber",
            value: value.clone(),
        },
    );
    draw(&mut cx, window);
    assert_eq!(value.get(), 1);
    cx.update(|cx| cx.set_global(Settings(2)));
    draw(&mut cx, window);
    assert_eq!(value.get(), 2);
}

#[derive(Default)]
struct ResourceProbe {
    handle: Cell<Option<Resource<i32, std::io::Error>>>,
    attempts: Cell<usize>,
    value: Cell<Option<i32>>,
}

#[rake::component]
fn Fetcher(
    cx: &mut Scope,
    runtime: rake::runtime::RuntimeContext,
    input: i32,
    probe: Rc<ResourceProbe>,
) -> impl IntoElement {
    let attempts = probe.clone();
    let resource = cx.use_resource(&runtime, input, move |input, _| {
        attempts.attempts.set(attempts.attempts.get() + 1);
        async move { Ok::<_, std::io::Error>(input * 2) }
    });
    probe.handle.replace(Some(resource));
    resource
        .read(cx, |state| {
            if let rake::loading::LoadStatus::Ready(value) = state {
                probe.value.set(Some(*value));
            }
        })
        .unwrap();
    gpui::div()
}

#[test]
fn resources_reuse_the_managed_loader_and_replace_it_on_input_changes() {
    let mut cx = gpui::TestAppContext::single();
    let probe = Rc::new(ResourceProbe::default());
    let window = cx.add_window(|_, cx| {
        let runtime = Runtime::new(
            RouterConfig::default(),
            Services::new(
                HttpClient::new(HttpConfig::new()).unwrap(),
                Collector::default(),
                ServiceRegistry::new(),
            ),
        );
        let context = runtime.context();
        let driver = cx.spawn(async move |_, _| {
            let _ = runtime.run().await;
        });
        Host {
            props: Fetcher {
                id: "fetcher",
                runtime: context,
                input: 2,
                probe: probe.clone(),
            },
            render: IntoElement::into_any_element,
            visible: true,
            _driver: Some(driver),
        }
    });
    draw(&mut cx, window);
    draw(&mut cx, window);
    assert_eq!(probe.value.get(), Some(4));
    assert_eq!(probe.attempts.get(), 1);
    refresh(&mut cx, window);
    assert_eq!(probe.attempts.get(), 1);
    change(&mut cx, window, |host| host.props.input = 3);
    draw(&mut cx, window);
    assert_eq!(probe.value.get(), Some(6));
    assert_eq!(probe.attempts.get(), 2);
    cx.update(|cx| probe.handle.get().unwrap().retry(cx).unwrap());
    draw(&mut cx, window);
    assert_eq!(probe.attempts.get(), 3);
    change(&mut cx, window, |host| host.visible = false);
    draw(&mut cx, window);
    assert!(
        cx.update(|cx| probe.handle.get().unwrap().retry(cx))
            .is_err()
    );
}

#[rake::component(memo)]
fn MemoChild(
    cx: &mut Scope,
    input: i32,
    renders: Rc<Cell<usize>>,
    setter: Rc<Cell<Option<State<i32>>>>,
) -> impl IntoElement {
    let (count, update) = cx.use_state(|| input);
    setter.set(Some(update));
    renders.set(renders.get() + 1);
    gpui::div().child(count.to_string())
}

fn cached<P: FunctionComponent>(props: P) -> gpui::AnyElement {
    let style = gpui::div()
        .w(gpui::px(100.))
        .h(gpui::px(40.))
        .style()
        .clone();
    rake::component::ComponentElement::new(props)
        .cached(style)
        .into_any_element()
}

#[test]
fn memo_components_skip_equal_props_with_intrinsic_or_explicit_layout() {
    for explicit in [false, true] {
        let mut cx = gpui::TestAppContext::single();
        let renders = Rc::new(Cell::new(0));
        let setter = Rc::new(Cell::new(None));
        let window = mount(
            &mut cx,
            MemoChild {
                id: "memo",
                input: 1,
                renders: renders.clone(),
                setter: setter.clone(),
            },
        );
        change(&mut cx, window, |host| {
            if explicit {
                host.render = cached;
            }
        });
        let before = renders.get();
        refresh(&mut cx, window);
        assert_eq!(renders.get(), before);
        cx.update(|cx| setter.get().unwrap().set(cx, 5).unwrap());
        draw(&mut cx, window);
        assert!(renders.get() > before);
        let before = renders.get();
        change(&mut cx, window, |host| host.props.input = 2);
        assert!(renders.get() > before);
    }
}

#[test]
fn cached_siblings_skip_rendering_when_only_one_component_changes() {
    let mut cx = gpui::TestAppContext::single();
    let renders = [Rc::new(Cell::new(0)), Rc::new(Cell::new(0))];
    let setters = [Rc::new(Cell::new(None)), Rc::new(Cell::new(None))];
    let window = cx.add_window(|_, _| Host {
        props: [0, 1].map(|index| MemoChild {
            id: ("counter", index),
            input: 0,
            renders: renders[index].clone(),
            setter: setters[index].clone(),
        }),
        render: |props| gpui::div().children(props.map(cached)).into_any_element(),
        visible: true,
        _driver: None,
    });
    draw(&mut cx, window);
    let before = renders.each_ref().map(|count| count.get());

    cx.update(|cx| setters[0].get().unwrap().set(cx, 7).unwrap());
    draw(&mut cx, window);
    assert_eq!(renders[0].get(), before[0] + 1);
    assert_eq!(renders[1].get(), before[1]);
    assert_eq!(
        cx.update(|cx| setters[0].get().unwrap().read(cx, |n| *n)),
        Ok(7)
    );
    assert_eq!(
        cx.update(|cx| setters[1].get().unwrap().read(cx, |n| *n)),
        Ok(0)
    );

    refresh(&mut cx, window);
    assert_eq!(renders[0].get(), before[0] + 1);
    assert_eq!(renders[1].get(), before[1]);
}
