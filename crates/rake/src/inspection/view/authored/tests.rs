//! Covers: automatic view trees, source metadata, stable keyed lifetimes and opt-in sampling.
//! Real: public view macro, GPUI layout/paint/input and inspection store.
//! Doubles: GPUI TestPlatform. Requires: view-test-support; no GPU/display execution.

use std::{cell::Cell, rc::Rc};

use gpui::{prelude::*, px};

use crate::{
    inspection::{Snapshot, Store},
    router::WindowId,
};

#[derive(IntoElement)]
struct Row {
    id: gpui::ElementId,
}

impl gpui::RenderOnce for Row {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        crate::view! { <button id={self.id} class="px-2 py-1 hover:opacity-50">"Private label"</button> }
    }
}

struct Fixture {
    store: Store,
    rows: Vec<usize>,
    selected: bool,
    calls: Rc<Cell<usize>>,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        crate::view! {
            <section id="root" class="size-full flex flex-col">
                <div id="styled" class={if self.selected { "p-4" } else { "p-2" }}
                    style={|s| { self.calls.set(self.calls.get() + 1); s.w(px(120.)) }}>
                    "Private text content"
                </div>
                <p id="paragraph" class="text-sm"><strong class="font-bold">"Private inline content"</strong></p>
                for row in &self.rows {
                    <Row id={gpui::ElementId::from(("row", *row))}/>
                }
                {gpui::div().id("opaque").size(px(10.))}
            </section>
        }.inspect("renamed-root").root(self.store.clone(), 0, WindowId(1))
    }
}

fn named<'a>(snapshot: &'a Snapshot, semantic: &str) -> &'a crate::inspection::Element {
    snapshot
        .elements
        .iter()
        .find(|element| element.semantic_id.as_deref() == Some(semantic))
        .unwrap()
}

#[test]
fn automatic_boundaries_capture_styles_sources_and_inline_run_metadata_without_text() {
    let mut app = gpui::TestAppContext::single();
    let store = Store::default();
    store.attach();
    let calls = Rc::new(Cell::new(0));
    let (_, cx) = app.add_window_view(|_, _| Fixture {
        store: store.clone(),
        rows: vec![1, 2],
        selected: false,
        calls: calls.clone(),
    });
    let before = calls.get();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert_eq!(
        calls.get(),
        before + 1,
        "style metadata must not execute a closure twice"
    );
    let snapshot = store.snapshot();
    let root = named(&snapshot, "renamed-root");
    assert_eq!(
        snapshot
            .elements
            .iter()
            .filter(|e| e.view.as_ref().is_some_and(|v| v.tag == "section"))
            .count(),
        1,
        "manual inspect renames the automatic root"
    );
    let styled = named(&snapshot, "styled");
    assert_eq!(styled.parent.as_deref(), Some(root.id.as_str()));
    assert!(styled.resolved.is_some());
    let source = styled.source.as_ref().unwrap();
    assert_eq!(source.file, file!());
    let sources = &styled.view.as_ref().unwrap().style_sources;
    assert_eq!(
        sources
            .iter()
            .map(|source| source.attribute.as_str())
            .collect::<Vec<_>>(),
        ["class", "style"]
    );
    assert!(sources[0].expression.contains("self.selected"));
    assert_eq!(sources[0].source.line, source.line);
    assert!(sources[1].source.line > sources[0].source.line);
    assert_eq!(sources[1].source.file, file!());
    let paragraph = named(&snapshot, "paragraph");
    assert!(
        paragraph
            .view
            .as_ref()
            .unwrap()
            .style_sources
            .iter()
            .any(|source| source.scope == "strong" && source.expression.contains("font-bold"))
    );
    assert!(
        snapshot
            .elements
            .iter()
            .any(|e| e.view.as_ref().is_some_and(|v| v.tag == "Row"))
    );
    assert!(
        snapshot
            .elements
            .iter()
            .any(|e| e.view.as_ref().is_some_and(|v| v.tag == "button") && e.resolved.is_some())
    );
    assert!(
        snapshot
            .elements
            .iter()
            .any(|e| e.view.as_ref().is_some_and(|v| v.tag == "Text"))
    );
    assert!(
        snapshot
            .elements
            .iter()
            .any(|e| e.view.as_ref().is_some_and(|v| v.tag == "Expression"))
    );
    assert!(
        snapshot.elements.iter().all(|e| e.stable_id.is_some()),
        "unique macro sites and keyed components must be stable"
    );
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains("Private text content"));
    assert!(!serialized.contains("Private inline content"));
    assert!(
        snapshot.elements.iter().any(|element| element
            .accessibility
            .as_ref()
            .is_some_and(|a11y| a11y.label.as_deref() == Some("Private label"))),
        "accessible control names are captured, not raw text runs"
    );
}

#[test]
fn keyed_components_and_descendants_survive_reorder_and_style_changes_but_expire_on_removal() {
    let mut app = gpui::TestAppContext::single();
    let store = Store::default();
    store.attach();
    let (view, cx) = app.add_window_view(|_, _| Fixture {
        store: store.clone(),
        rows: vec![1, 2],
        selected: false,
        calls: Rc::new(Cell::new(0)),
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let identities = |snapshot: &Snapshot| {
        snapshot
            .elements
            .iter()
            .filter(|e| e.semantic_id.as_deref() == Some("row-1"))
            .map(|e| {
                (
                    e.view.as_ref().unwrap().tag.clone(),
                    e.stable_id.clone().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };
    let initial = identities(&store.snapshot());
    assert_eq!(
        initial.len(),
        2,
        "the component boundary and its button are both captured"
    );
    view.update(cx, |view, cx| {
        view.rows.reverse();
        view.selected = true;
        cx.notify();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert_eq!(identities(&store.snapshot()), initial);
    view.update(cx, |view, cx| {
        view.rows.retain(|row| *row != 1);
        cx.notify();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(identities(&store.snapshot()).is_empty());
    view.update(cx, |view, cx| {
        view.rows.push(1);
        cx.notify();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    for (_, id) in identities(&store.snapshot()) {
        assert!(!initial.iter().any(|(_, old)| *old == id));
    }
}

#[test]
fn authored_metadata_is_bounded_without_evaluating_or_dropping_its_boundary() {
    let source = std::panic::Location::caller();
    let expression = "x".repeat(2048);
    let expression: &'static str = Box::leak(expression.into_boxed_str());
    let sites: &'static [super::StyleSite] = Box::leak(
        (0..32)
            .map(|_| super::StyleSite {
                scope: "div",
                attribute: "style",
                expression,
                source,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let definition = super::Definition {
        tag: "div",
        site: "test",
        source,
        styles: sites,
        handlers: &[],
    };
    let mut truncated = false;
    let metadata = definition.capture(&mut truncated);
    assert!(truncated);
    assert!(!metadata.style_sources.is_empty());
    assert!(metadata.style_sources.len() < sites.len());
    assert!(
        metadata
            .style_sources
            .iter()
            .all(|source| source.expression.len() <= 512)
    );
    assert!(serde_json::to_vec(&metadata).unwrap().len() < 4096);
}

#[test]
fn views_without_an_inspection_host_do_not_sample_styles() {
    struct WithoutHost;
    impl Render for WithoutHost {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            let mut element = crate::view! { <div class="p-2">"Normal application"</div> };
            element.style_reader = Some(|_| panic!("disabled inspection must not sample styles"));
            element
        }
    }
    let mut app = gpui::TestAppContext::single();
    let (_, cx) = app.add_window_view(|_, _| WithoutHost);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
}

#[test]
fn component_properties_do_not_require_clone_debug_or_gpui_key_conversion() {
    struct CustomId(usize);
    #[derive(IntoElement)]
    struct CustomComponent {
        id: CustomId,
    }
    impl gpui::RenderOnce for CustomComponent {
        fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
            gpui::div()
        }
    }
    let component = crate::view! { <CustomComponent id={CustomId(17)}/> };
    assert_eq!(component.id.0, 17);
    let element = component.into_element();
    assert!(
        element.view_key.is_none(),
        "unknown application key types must remain ambiguous"
    );
}

struct AccessibleFixture {
    store: Store,
    scope: gpui::FocusHandle,
    disabled: bool,
    clicks: Rc<Cell<usize>>,
}

impl Render for AccessibleFixture {
    fn render(&mut self, _: &mut gpui::Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let clicks = self.clicks.clone();
        crate::view! {
            <section id="accessibility-root" role={gpui::accesskit::Role::Group} aria-label="Tools" focus={&self.scope} tab-navigation={true}>
                <button id="save" aria-label="Save changes" aria-description="Writes document" disabled={self.disabled} tabindex={3}
                    on:click={move |_, _, _| clicks.set(clicks.get() + 1)}>"Sensitive body text"</button>
                <button id="cancel" aria-label="Cancel" tabindex={0}/>
            </section>
        }.root(self.store.clone(), 0, WindowId(1))
    }
}

#[test]
fn accessible_roles_labels_actions_focus_order_and_handler_sites_follow_real_gpui_state() {
    let mut app = gpui::TestAppContext::single();
    let store = Store::default();
    store.attach();
    let clicks = Rc::new(Cell::new(0));
    let (view, cx) = app.add_window_view(|_, cx| AccessibleFixture {
        store: store.clone(),
        scope: cx.focus_handle().tab_stop(false),
        disabled: false,
        clicks: clicks.clone(),
    });
    let draw = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        })
    };
    draw(cx);
    let snapshot = store.snapshot();
    let save = named(&snapshot, "save");
    let a11y = save.accessibility.as_ref().unwrap();
    assert_eq!(a11y.role.as_deref(), Some("Button"));
    assert_eq!(a11y.label.as_deref(), Some("Save changes"));
    assert_eq!(a11y.description.as_deref(), Some("Writes document"));
    assert_eq!(a11y.tab_index, Some(3));
    assert_eq!(a11y.tab_stop, Some(true));
    assert_eq!(a11y.captured_tab_order, Some(2));
    assert_eq!(
        named(&snapshot, "cancel")
            .accessibility
            .as_ref()
            .unwrap()
            .captured_tab_order,
        Some(1)
    );
    assert!(save.navigation.focus);
    assert!(a11y.actions.iter().any(|action| action == "Click"));
    assert_eq!(save.handlers[0].name, "click");
    assert_eq!(save.handlers[0].origin, "view");
    assert!(save.handlers[0].source.line > 0);
    assert!(
        !serde_json::to_string(&snapshot)
            .unwrap()
            .contains("Sensitive body text")
    );
    store
        .navigate(
            &save.id,
            &snapshot.capture,
            crate::inspection::NavigationAction::Focus,
        )
        .unwrap();
    draw(cx);
    let focused = store.snapshot();
    assert_eq!(
        named(&focused, "save")
            .accessibility
            .as_ref()
            .unwrap()
            .focused,
        Some(true)
    );
    assert_eq!(
        named(&focused, "accessibility-root")
            .accessibility
            .as_ref()
            .unwrap()
            .contains_focus,
        Some(true)
    );
    view.update(cx, |view, cx| {
        view.disabled = true;
        cx.notify();
    });
    draw(cx);
    let disabled = store.snapshot();
    let save = named(&disabled, "save");
    let a11y = save.accessibility.as_ref().unwrap();
    assert!(a11y.disabled);
    assert!(!a11y.focusable);
    assert_eq!(a11y.tab_stop, Some(false));
    assert!(a11y.actions.is_empty());
    assert!(!save.navigation.focus);
    assert_eq!(clicks.get(), 0, "inspection never invokes event handlers");
}
