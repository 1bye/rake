//! Macro-authored Elements screen; remote work is owned by its Rake entry activity.

use crate::input::FilterInput;
use crate::{Connection, ConnectionConfig, Inspector, Update};
use rake::control::{Query, Reply};
use rake::gpui::{self, prelude::*, *};
use rake::prelude::*;
use rake::router::{MountError, RuntimeId};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Route)]
pub(crate) enum Destination {
    #[route(id = "elements", path = "/")]
    Elements,
}

struct Views;

struct Broker(Result<ConnectionConfig, String>);

struct ElementsView {
    model: Rc<RefCell<Inspector>>,
    config_error: Option<String>,
    filter: Entity<FilterInput>,
    focus: FocusHandle,
    divider: FocusHandle,
    show_targets: bool,
    detail_tab: &'static str,
    split: f32,
    dragging: bool,
    light: bool,
    expanded_styles: std::collections::BTreeSet<String>,
}

impl RouteViews<Destination> for Views {
    fn mount(
        &self,
        _: &Destination,
        page: PageContext<Destination>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<AnyView, MountError> {
        window.set_window_title("Rake Inspector — Elements");
        #[cfg(not(target_family = "wasm"))]
        {
            let navigator = page.navigator();

            window.on_window_should_close(cx, move |_, _| {
                drop(navigator.quit(QuitOptions::default()));

                false
            });
        }
        let config = page
            .services()
            .get::<Broker>()
            .map_err(|error| MountError(error.to_string()))?;

        let model = Rc::new(RefCell::new(Inspector::new(
            config
                .0
                .as_ref()
                .ok()
                .and_then(|config| config.instance.clone()),
        )));

        let owned = model.clone();

        let view = cx.new(|cx| {
            let weak = cx.weak_entity();
            let state = model.clone();

            let filter = cx.new(|cx| {
                FilterInput::new(
                    move |text, cx| {
                        state.borrow_mut().filter = text;
                        let _ = weak.update(cx, |_, cx| cx.notify());
                    },
                    cx,
                )
            });

            ElementsView {
                model,
                config_error: config.0.as_ref().err().cloned(),
                filter,
                focus: cx.focus_handle(),
                divider: cx.focus_handle(),
                show_targets: false,
                detail_tab: "Styles",
                split: 0.58,
                dragging: false,
                light: false,
                expanded_styles: std::collections::BTreeSet::new(),
            }
        });

        let focus = view.read(cx).focus.clone();

        window.focus(&focus, cx);
        let weak = view.downgrade();
        let context = window.to_async(cx);

        page.on_event(move |update: Update| {
            owned.borrow_mut().apply(update);
            let mut context = context.clone();

            // Publications run on the UI loop, after Rake validates the owner and run.
            context
                .update(|_, cx| weak.update(cx, |_, cx| cx.notify()))
                .map_err(|error| TaskError::Failed(error.to_string()))?
                .map_err(|error| TaskError::Failed(error.to_string()))?;

            Ok(())
        })
        .map_err(|error| MountError(error.to_string()))?;

        if let Ok(config) = &config.0 {
            let connection = Connection::new(config.clone()).map_err(MountError)?;
            let observed = view.read(cx).model.clone();

            page.activity("inspector.observe", move |run| {
                let connection = connection.clone();
                let observed = observed.clone();

                async move {
                    {
                        let mut model = observed.borrow_mut();

                        model.refresh = true;
                        model.detail_pending = model.selected.is_some() && model.detail.is_none();
                    }

                    loop {
                        let (epoch, disconnected) = {
                            let model = observed.borrow();

                            (model.epoch, model.disconnected)
                        };

                        if !disconnected {
                            let instances = connection.instances(run.cancellation()).await;

                            run.publish(Update::Instances(epoch, instances))?;
                            let target = {
                                let mut model = observed.borrow_mut();
                                let refresh = std::mem::take(&mut model.refresh);

                                model
                                    .runtime
                                    .map(|runtime| (model.epoch, runtime, !model.paused || refresh))
                            };

                            if let Some((epoch, runtime, refresh)) = target {
                                let selection = {
                                    let mut model = observed.borrow_mut();

                                    if std::mem::take(&mut model.detail_pending) {
                                        model.selected.clone()
                                    } else {
                                        None
                                    }
                                };

                                if let Some(id) = selection {
                                    let result = connection
                                        .query(
                                            runtime,
                                            Query::InspectElement { id: id.clone() },
                                            run.cancellation(),
                                        )
                                        .await
                                        .and_then(|reply| match reply {
                                            Reply::Element(element) => Ok(element),
                                            _ => Err("Unexpected element response.".into()),
                                        });

                                    run.publish(Update::Element(epoch, runtime, id, result))?;
                                }

                                if refresh {
                                    let result = connection
                                        .query(runtime, Query::InspectTree {}, run.cancellation())
                                        .await
                                        .and_then(|reply| match reply {
                                            Reply::Elements(tree)
                                                if tree.elements.len()
                                                    <= rake::inspection::MAX_ELEMENTS =>
                                            {
                                                Ok(tree)
                                            }
                                            _ => {
                                                Err("Unexpected or oversized element tree.".into())
                                            }
                                        });

                                    run.publish(Update::Tree(epoch, runtime, result))?;
                                }
                            }
                        }

                        rake::tasks::sleep(Duration::from_millis(500)).await;
                    }
                }
            })
            .map_err(|error| MountError(error.to_string()))?;
        }

        Ok(view.into())
    }
}

fn short_kind(kind: &str) -> String {
    kind.split_inclusive(['<', '>', ','])
        .map(|part| part.rsplit("::").next().unwrap_or(part))
        .collect()
}

fn style_summary(value: &serde_json::Value) -> String {
    if let Some(color) = value
        .get("Color")
        .filter(|color| color["tag"] == "Solid")
        .and_then(|color| color.get("solid"))
    {
        let components: Option<Vec<f64>> = ["h", "s", "l", "a"]
            .into_iter()
            .map(|name| {
                color[name]
                    .as_f64()
                    .filter(|value| (0.0..=1.0).contains(value))
            })
            .collect();

        if let Some(c) = components {
            return format!(
                "hsl({:.1} {:.1}% {:.1}% / {:.2})",
                c[0] * 360.,
                c[1] * 100.,
                c[2] * 100.,
                c[3]
            );
        }
    }

    if let Some(fields) = value.as_object() {
        let mut text = fields
            .iter()
            .take(4)
            .map(|(name, value)| {
                format!(
                    "{name}: {}",
                    if value.is_object() || value.is_array() {
                        "…".into()
                    } else {
                        style_summary(value)
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ");

        if fields.len() > 4 {
            text.push_str(", …");
        }

        format!("{{ {text} }}")
    } else {
        value
            .as_str()
            .map_or_else(|| value.to_string(), str::to_owned)
    }
}

fn property(label: &str, value: impl Into<SharedString>) -> impl IntoElement {
    let label = label.to_owned();
    let value = value.into();

    rake::view! {
        <div id={format!("property-{label}")} class="flex flex-row gap-3 px-3 py-2 border" style={|s| s.border_color(rgb(0x3c4043))}>
            <span style={|s| s.w(px(95.)).flex_shrink_0().text_color(rgb(0x9aa0a6))}>{label}</span>
            <span class="min-w-0">{value}</span>
        </div>
    }
}

impl ElementsView {
    fn detail_button(
        &self,
        tab: &'static str,
        blue: u32,
        muted: u32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        rake::view! {
            <button id={format!("detail-{tab}")} class="px-3" style={|s| s.h_full().text_color(rgb(if self.detail_tab == tab { blue } else { muted })).border_b(px(if self.detail_tab == tab { 2. } else { 0. })).border_color(rgb(blue))}
                onclick={cx.listener(move |this, _, _, cx| { this.detail_tab = tab; cx.notify(); })}>{tab}</button>
        }.inspect(format!("inspector.tab.{tab}"))
    }

    fn select(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.model.borrow_mut().select_element(id);
        self.expanded_styles.clear();
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn select_runtime(&mut self, runtime: RuntimeId, cx: &mut Context<Self>) {
        let mut model = self.model.borrow_mut();

        model.disconnected = false;
        model.select_runtime(Some(runtime));
        self.show_targets = false;
        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "tab" {
            if event.keystroke.modifiers.shift {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }

            window.prevent_default();
            cx.stop_propagation();
        }
    }
}

impl Render for ElementsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        let narrow = width < 720.;
        let background = if self.light { 0xffffff } else { 0x202124 };
        let surface = if self.light { 0xf1f3f4 } else { 0x292a2d };
        let ink = if self.light { 0x202124 } else { 0xe8eaed };
        let line = if self.light { 0xdadce0 } else { 0x3c4043 };
        let blue = if self.light { 0x1a73e8 } else { 0xa8c7fa };
        let muted = if self.light { 0x5f6368 } else { 0x9aa0a6 };
        let model = self.model.borrow();

        let target = model
            .instances
            .iter()
            .find(|instance| Some(instance.info.runtime_id) == model.runtime);

        let target_name = target.map_or_else(
            || "Select runtime".to_owned(),
            |instance| format!("{} · {}", instance.application_id, instance.info.platform),
        );

        let elements = model.visible_elements();
        let mut depths = BTreeMap::new();

        if let Some(tree) = &model.tree {
            for element in &tree.elements {
                depths.insert(
                    element.id.clone(),
                    element
                        .parent
                        .as_ref()
                        .and_then(|parent| depths.get(parent))
                        .copied()
                        .unwrap_or(0usize)
                        + usize::from(element.parent.is_some()),
                );
            }
        }

        let error = self.config_error.as_ref().or(model.error.as_ref()).cloned();
        let selected = model.detail.as_ref();
        let selected_id = model.selected.clone();
        let tree_count = model.tree.as_ref().map_or(0, |tree| tree.elements.len());
        let omitted = model.tree.as_ref().map_or(0, |tree| tree.omitted);

        let status = if model.disconnected {
            "Disconnected"
        } else if error.is_some() {
            "Connection error"
        } else if model.tree.is_some() {
            if model.paused { "Paused" } else { "Connected" }
        } else {
            "Connecting"
        };

        let tree_status = model.tree.as_ref().map_or_else(
            || "Waiting for capture".into(),
            |tree| {
                format!(
                    "{:?} · {tree_count} boundaries · {omitted} omitted · capture {}",
                    tree.status, tree.capture
                )
            },
        );

        let stale = model.stale();
        let current_tab = self.detail_tab;

        let tree = rake::view! {
            <section id="element-tree" class="flex flex-col min-h-0" style={|s| s.flex_1().overflow_y_scroll()} aria-label="Element tree">
                for element in elements {
                    <button id={format!("boundary-{}", element.id)} aria-label={format!("Inspect {}", element.semantic_id.as_deref().unwrap_or(&element.id))}
                        style={|s| s.w_full().h(px(25.)).flex_shrink_0().flex_row().items_center().justify_start().gap(px(5.)).pl(px(10. + depths.get(&element.id).copied().unwrap_or(0).min(16) as f32 * 14.)).pr(px(8.)).text_size(px(12.)).overflow_hidden().bg(rgb(if selected_id.as_ref() == Some(&element.id) { if self.light { 0xd3e3fd } else { 0x3c4b64 } } else { background })).hover(|s| s.bg(rgb(surface)))}
                        onclick={ { let id = element.id.clone(); cx.listener(move |this, _, window, cx| this.select(&id, window, cx)) } }>
                        <span style={|s| s.text_color(rgb(muted))}>"‹"</span>
                        <span style={|s| s.text_color(rgb(if self.light { 0x881280 } else { 0xc58af9 }))}>{short_kind(&element.kind)}</span>
                        <span style={|s| s.text_color(rgb(if self.light { 0x196127 } else { 0xa8dab5 }))}>{element.semantic_id.clone().unwrap_or_else(|| element.id.clone())}</span>
                        <span style={|s| s.text_color(rgb(muted))}>"›"</span>
                    </button>
                }
                if tree_count == 0 {
                    <div class="p-6" style={|s| s.text_color(rgb(muted))}>{if model.tree.is_some() { "No inspection boundaries available. Enable capture in the target application." } else { "Connect to a running Rake application to inspect its elements." }}</div>
                } else if model.visible_elements().is_empty() {
                    <div class="p-6" style={|s| s.text_color(rgb(muted))}>"No matching boundaries."</div>
                }
            </section>
        }.inspect("inspector.tree");

        let details = rake::view! {
            <section id="element-details" class="flex flex-col flex-1 min-w-0 min-h-0" style={|s| s.overflow_y_scroll()} aria-label="Element details">
                <div class="flex flex-row" style={|s| s.h(px(30.)).flex_shrink_0().bg(rgb(surface)).border_b_1().border_color(rgb(line))}>
                    for tab in ["Styles", "Layout", "Source"] {
                        <div id={format!("tab-container-{tab}")} style={|s| s.h_full()}>{self.detail_button(tab, blue, muted, cx)}</div>
                    }
                </div>
                if stale {
                    <div class="px-3 py-2" style={|s| s.text_color(rgb(0xfdd663)).bg(rgb(0x3b3321))}>"Capture replaced. Details are frozen; select a current boundary."</div>
                }
                if let Some(error) = &model.lookup_error { <p class="p-3" style={|s| s.text_color(rgb(0xf28b82))}>{error.clone()}</p> }
                if let Some(element) = selected {
                    <div class="p-3 flex flex-col gap-1" style={|s| s.border_b_1().border_color(rgb(line))}>
                        <span style={|s| s.text_color(rgb(blue))}>{short_kind(&element.kind)}</span>
                        <span>{element.semantic_id.clone().unwrap_or_else(|| "No semantic ID".into())}</span>
                        <span style={|s| s.text_color(rgb(muted))}>{element.id.clone()}</span>
                    </div>
                    if current_tab == "Styles" {
                        <div class="p-3 flex flex-col gap-2">
                            <span style={|s| s.text_color(rgb(muted))}>"Authored base styles"</span>
                            if let Some(styles) = element.styles.as_ref().and_then(|styles| styles.as_object()) {
                                for (name, value) in styles {
                                    <div id={format!("style-{name}")} class="flex flex-col gap-1">
                                        <div class="flex flex-row gap-2">
                                            <span style={|s| s.text_color(rgb(if self.light { 0x881280 } else { 0xc58af9 }))}>{format!("{name}:")}</span>
                                            if value.is_object() || value.is_array() {
                                                <button id={format!("expand-{name}")} aria-label={format!("Expand {name} metadata")}
                                                    style={|s| s.justify_start().text_color(rgb(if self.light { 0x196127 } else { 0xa8dab5 }))}
                                                    onclick={ { let name = name.clone(); cx.listener(move |this, _, _, cx| { if !this.expanded_styles.remove(&name) { this.expanded_styles.insert(name.clone()); } cx.notify(); }) } }>{format!("{} {}", if self.expanded_styles.contains(name) { "-" } else { "+" }, style_summary(value))}</button>
                                            } else {
                                                <span style={|s| s.text_color(rgb(if self.light { 0x196127 } else { 0xa8dab5 }))}>{style_summary(value)}</span>
                                            }
                                        </div>
                                        if self.expanded_styles.contains(name) {
                                            <div class="p-2" style={|s| s.bg(rgb(surface))}>{serde_json::to_string_pretty(value).unwrap_or_default()}</div>
                                        }
                                    </div>
                                }
                            } else { <p>"Authored style refinements unavailable."</p> }
                        </div>
                    }
                    if current_tab != "Source" {
                        <div class="px-3 py-2" style={|s| s.bg(rgb(surface))}>"Layout · window logical pixels"</div>
                        if let Some(layout) = element.layout {
                            <div class="m-4 p-4 flex flex-col gap-3 border" style={|s| s.border_color(rgb(muted)).items_center()}>
                                <span style={|s| s.text_color(rgb(muted))}>{format!("x: {} · y: {}", layout.x, layout.y)}</span>
                                <div class="p-4 border" style={|s| s.border_color(rgb(blue)).bg(rgb(surface))}>{format!("{} × {}", layout.width, layout.height)}</div>
                                <span style={|s| s.text_color(rgb(muted))}>"Measured bounds before clipping"</span>
                            </div>
                        } else { <p class="p-3">"Layout unavailable."</p> }
                    }
                    if current_tab == "Source" {
                        if let Some(source) = &element.source {
                            {property("Source file", source.file.clone())}
                            {property("Line", source.line.to_string())}
                            {property("Column", source.column.to_string())}
                        } else { <p class="p-3">"Source location unavailable."</p> }
                        {property("Type", element.kind.clone())}
                        {property("Parent", element.parent.clone().unwrap_or_else(|| "Inspection root".into()))}
                        {property("Entry", element.entry_id.map_or_else(|| "Workspace shell".into(), |id| id.to_string()))}
                    }
                    <p class="p-3" style={|s| s.text_color(rgb(muted))}>{if element.metadata_truncated { "Some metadata exceeded capture limits." } else { "Read-only inspection metadata." }}</p>
                } else {
                    <div class="p-6" style={|s| s.text_color(rgb(muted))}>"Select a view to inspect its styles, layout and source."</div>
                }
            </section>
        }.inspect("inspector.details");

        rake::view! {
            <div id="inspector" class="size-full flex flex-col min-h-0" style={|s| s.bg(rgb(background)).text_color(rgb(ink)).text_size(px(12.)).track_focus(&self.focus)}
                onkeydown={cx.listener(Self::key)}
                onmousemove={cx.listener(|this, event: &MouseMoveEvent, window, cx| { if this.dragging { if event.dragging() { let width = f32::from(window.viewport_size().width); this.split = (f32::from(event.position.x) / width).clamp(0.2, 0.8); cx.notify(); } else { this.dragging = false; } } })}
                onmouseup={cx.listener(|this, _, _, _| this.dragging = false)}>
                <div class="flex flex-row items-center gap-3 px-3" style={|s| s.h(px(35.)).flex_shrink_0().bg(rgb(surface)).border_b_1().border_color(rgb(line))}>
                    <span style={|s| s.text_color(rgb(muted))}>"Rake Inspector"</span>
                    <div class="px-3 flex items-center" style={|s| s.h_full().border_b_2().border_color(rgb(blue)).text_color(rgb(blue))}>"Elements"</div>
                    <span class="flex-1"/>
                    <button id="theme" aria-label="Toggle theme" onclick={cx.listener(|this, _, _, cx| { this.light = !this.light; this.filter.update(cx, |input, cx| input.set_light(this.light, cx)); cx.notify(); })}>"◐"</button>
                </div>
                <div class="flex flex-row items-center gap-3 px-3" style={|s| s.h(px(32.)).flex_shrink_0().border_b_1().border_color(rgb(line))}>
                    <button id="targets" aria-label="Select target runtime" style={|s| s.text_color(rgb(blue))} onclick={cx.listener(|this, _, _, cx| { this.show_targets = !this.show_targets; cx.notify(); })}>{target_name}</button>
                    <span class="flex-1"/>
                    <span style={|s| s.text_color(rgb(muted))}>{status}</span>
                    <button id="disconnect" onclick={cx.listener(|this, _, _, cx| { let mut model = this.model.borrow_mut(); model.disconnected = !model.disconnected; let runtime = model.runtime; model.select_runtime(runtime); cx.notify(); })}>{if model.disconnected { "Reconnect" } else { "Disconnect" }}</button>
                </div>
                if self.show_targets {
                    <section class="flex flex-col p-2 gap-1" style={|s| s.max_h(px(160.)).overflow_y_scroll().bg(rgb(surface))}>
                        for instance in &model.instances {
                            <button id={format!("runtime-{}", instance.info.runtime_id)} class="px-3 py-1" onclick={ { let runtime = instance.info.runtime_id; cx.listener(move |this, _, _, cx| this.select_runtime(runtime, cx)) } }>{format!("{} · {} · {}", instance.application_id, instance.info.platform, instance.info.runtime_id)}</button>
                        }
                        if model.instances.is_empty() { <p>"No runtimes advertised by this broker."</p> }
                    </section>
                }
                <div class="flex flex-row items-center gap-3 px-2" style={|s| s.h(px(34.)).flex_shrink_0().border_b_1().border_color(rgb(line))}>
                    <div style={|s| s.w(px(300.)).max_w_full()}>{self.filter.clone()}</div>
                    <button id="pause" onclick={cx.listener(|this, _, _, cx| { let mut model = this.model.borrow_mut(); model.paused = !model.paused; cx.notify(); })}>{if model.paused { "Resume" } else { "Pause" }}</button>
                    <button id="refresh" onclick={cx.listener(|this, _, _, cx| { this.model.borrow_mut().refresh = true; cx.notify(); })}>"Refresh"</button>
                </div>
                if let Some(error) = error { <div class="px-3 py-2" style={|s| s.bg(rgb(0x3c2021)).text_color(rgb(0xf28b82))}>{error}</div> }
                <div class="px-3 py-1" style={|s| s.text_color(rgb(muted)).border_b_1().border_color(rgb(line))}>{tree_status}</div>
                <div class="flex flex-1 min-h-0 min-w-0" style={|s| if narrow { s.flex_col() } else { s.flex_row() }}>
                    <div class="flex flex-col min-h-0" style={|s| if narrow { s.h(px(260.)).w_full().flex_shrink_0() } else { s.w(px((width * self.split).clamp(180., (width - 260.).max(180.)))) .flex_shrink_0() }}>
                        {tree}
                    </div>
                    if !narrow {
                        <div id="divider" tabindex={0} aria-label="Resize inspection panes" style={|s| s.w(px(5.)).h_full().flex_shrink_0().border_l_1().border_color(rgb(line)).cursor(CursorStyle::ResizeColumn).track_focus(&self.divider)}
                            onmousedown={cx.listener(|this, _, window, cx| { this.dragging = true; window.focus(&this.divider, cx); })}
                            onkeydown={cx.listener(|this, event: &KeyDownEvent, _, cx| { match event.keystroke.key.as_str() { "left" => this.split = (this.split - 0.025).max(0.2), "right" => this.split = (this.split + 0.025).min(0.8), "home" => this.split = 0.2, "end" => this.split = 0.8, _ => return } cx.stop_propagation(); cx.notify(); })}/>
                    }
                    {details}
                </div>
                <div class="px-3 py-1" style={|s| s.flex_shrink_0().bg(rgb(surface)).border_t_1().border_color(rgb(line)).text_color(rgb(muted))}>{model.tree.as_ref().map_or_else(|| "Rake views · shared inspection protocol".into(), |tree| tree.coverage.clone())}</div>
            </div>
        }.inspect("inspector.workspace")
    }
}

pub(crate) fn application() -> rake::App<Destination> {
    #[cfg(target_family = "wasm")]
    let base = if web_sys::window()
        .and_then(|window| window.location().pathname().ok())
        .is_some_and(|path| path.starts_with("/__rake/elements"))
    {
        "/__rake/elements/"
    } else {
        "/"
    };

    #[cfg(not(target_family = "wasm"))]
    let base = "/";

    rake::App::new("dev.rake.inspector")
        .routes(Views)
        .initial(Destination::Elements)
        .workspace_chrome(false)
        .web(
            WebConfig::new()
                .base_path(base)
                .expect("constant Inspector base path"),
        )
        .control(rake::control::ControlConfig::FromEnvironment)
        .service(Broker(ConnectionConfig::from_host()))
        .expect("one inspector connection service")
        .window_options(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(120.), px(80.)),
                size(px(1180.), px(800.)),
            ))),
            ..WindowOptions::default()
        })
}

#[cfg(test)]
mod tests {
    use super::property;
    use rake::gpui::{Element, IntoElement};

    #[test]
    fn source_properties_have_distinct_stable_sibling_identities() {
        let mut ids = std::collections::HashSet::new();

        for label in ["Source file", "Line", "Column", "Type", "Parent", "Entry"] {
            let element = property(label, "value").into_element();

            assert!(
                ids.insert(Element::id(&element).expect("property identity")),
                "duplicate {label}"
            );
        }
    }
}
