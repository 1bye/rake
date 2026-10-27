//! Macro-authored Elements screen; remote work is owned by its Rake entry activity.

use crate::appearance::{CODE_FONT, Icon, Palette, icon};
use crate::input::FilterInput;
use crate::{Connection, ConnectionConfig, Inspector, Update};
use crate::{styles, tree};
use rake::control::{Query, Reply};
use rake::gpui::{self, prelude::*, *};
use rake::prelude::*;
use rake::router::{MountError, RuntimeId};
use std::cell::RefCell;
use std::collections::BTreeSet;
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
    style_filter: Entity<FilterInput>,
    style_query: String,
    show_find: bool,
    show_coverage: bool,
    collapsed: BTreeSet<String>,
    capture: Option<(RuntimeId, String)>,
    breadcrumbs: Vec<(String, String)>,
    breadcrumb_scroll: ScrollHandle,
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
            let style_owner = weak.clone();
            let state = model.clone();

            let filter = cx.new(|cx| {
                FilterInput::new(
                    "Find elements",
                    "Find by ID, type or source",
                    move |text, cx| {
                        state.borrow_mut().filter = text;
                        let _ = weak.update(cx, |this: &mut ElementsView, cx| {
                            this.collapsed.clear();
                            cx.notify();
                        });
                    },
                    cx,
                )
            });

            let style_filter = cx.new(|cx| {
                FilterInput::new(
                    "Filter styles",
                    "Filter properties or values",
                    move |text, cx| {
                        let _ = style_owner.update(cx, |this: &mut ElementsView, cx| {
                            this.style_query = text;
                            cx.notify();
                        });
                    },
                    cx,
                )
            });

            ElementsView {
                model,
                config_error: config.0.as_ref().err().cloned(),
                filter,
                style_filter,
                style_query: String::new(),
                show_find: false,
                show_coverage: false,
                collapsed: BTreeSet::new(),
                capture: None,
                breadcrumbs: Vec::new(),
                breadcrumb_scroll: ScrollHandle::new(),
                focus: cx.focus_handle(),
                divider: cx.focus_handle(),
                show_targets: false,
                detail_tab: "Styles",
                split: 0.55,
                dragging: false,
                light: true,
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

fn property(label: &str, value: impl Into<SharedString>, colors: Palette) -> impl IntoElement {
    let label = label.to_owned();
    let value = value.into();

    rake::view! {
        <div id={format!("property-{label}")} class="flex flex-row gap-3 px-3 py-1" style={|s| s.border_b_1().border_color(rgb(colors.line))}>
            <span style={|s| s.w(px(86.)).flex_shrink_0().text_color(rgb(colors.muted))}>{label}</span>
            <span class="min-w-0" style={|s| s.font_family(CODE_FONT).text_size(px(11.))}>{value}</span>
        </div>
    }
}

fn source_label(element: &rake::inspection::Element) -> String {
    element.source.as_ref().map_or_else(
        || "Source unavailable".into(),
        |source| {
            format!(
                "{}:{}",
                source.file.rsplit('/').next().unwrap_or(&source.file),
                source.line
            )
        },
    )
}

impl ElementsView {
    fn detail_button(
        &self,
        tab: &'static str,
        colors: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        rake::view! {
            <button id={format!("detail-{tab}")} class="px-3" style={|s| s.h_full().text_color(rgb(if self.detail_tab == tab { colors.accent } else { colors.muted })).border_b(px(if self.detail_tab == tab { 2. } else { 0. })).border_color(rgb(colors.accent)).hover(|s| s.bg(rgb(colors.hover)))}
                on:click={cx.listener(move |this, _, _, cx| { this.detail_tab = tab; cx.notify(); })}>{tab}</button>
        }.inspect(format!("inspector.tab.{tab}"))
    }

    fn select(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let mut model = self.model.borrow_mut();

        if let Some(tree) = &model.tree {
            self.breadcrumbs = tree::ancestors(&tree.elements, id)
                .iter()
                .map(|element| {
                    let label = format!(
                        "{}{}",
                        tree::tag(&element.kind),
                        element
                            .semantic_id
                            .as_ref()
                            .map_or_else(String::new, |id| format!("#{id}"))
                    );

                    (element.id.clone(), label)
                })
                .collect();
        }

        model.select_element(id);
        self.breadcrumb_scroll
            .scroll_to_item(self.breadcrumbs.len().saturating_sub(1));
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

    fn toggle_find(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find = !self.show_find;

        if self.show_find {
            self.filter.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.filter.update(cx, |input, cx| input.reset(cx));
            self.model.borrow_mut().filter.clear();
            self.collapsed.clear();
        }

        cx.notify();
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke.key;
        let modifiers = event.keystroke.modifiers;

        if key == "f" && (modifiers.platform || modifiers.control) {
            self.show_find = true;
            self.filter.update(cx, |input, cx| input.focus(window, cx));
            window.prevent_default();
            cx.stop_propagation();
            cx.notify();
        } else if key == "escape" {
            self.show_targets = false;
            self.show_coverage = false;
            cx.notify();
        } else if key == "tab" && !modifiers.platform && !modifiers.control && !modifiers.alt {
            if modifiers.shift {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }

            window.prevent_default();
            cx.stop_propagation();
        }
    }

    fn tree_row(
        &self,
        row: tree::Row,
        element: &rake::inspection::Element,
        colors: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.model.borrow().selected.as_ref() == Some(&element.id);
        let kind = tree::tag(&element.kind);

        let flex = element
            .styles
            .as_ref()
            .is_some_and(|styles| styles["display"] == "Flex");

        rake::view! {
            <div id={format!("line-{}-{}", row.index, row.closing)} class="flex flex-row items-center min-w-0" style={|s| s.h(px(20.)).min_w_full().flex_shrink_0().pl(px(6.)).bg(rgb(if selected { colors.selection } else { colors.background })).hover(|s| s.bg(rgb(if selected { colors.selection } else { colors.hover })))}>
                for depth in 0..row.depth.min(16) {
                    <span id={format!("indent-{depth}")} style={|s| s.w(px(12.)).h_full().flex_shrink_0().border_l_1().border_color(rgb(colors.guide))}/>
                }
                if row.branch && !row.closing {
                    <button id={format!("disclose-{}", row.index)} aria-label={format!("{} {}", if row.collapsed { "Expand" } else { "Collapse" }, element.semantic_id.as_deref().unwrap_or(&element.id))}
                        style={|s| s.w(px(14.)).h_full().flex_shrink_0()}
                        on:click={ { let id = element.id.clone(); cx.listener(move |this, _, _, cx| { if !this.collapsed.remove(&id) { this.collapsed.insert(id.clone()); } cx.stop_propagation(); cx.notify(); }) } }>
                        {icon(if row.collapsed { Icon::Right } else { Icon::Down }, colors.muted)}
                    </button>
                } else { <span style={|s| s.w(px(14.)).flex_shrink_0()}/> }
                <button id={format!("select-{}-{}", row.index, row.closing)} aria-label={format!("Inspect {}", element.semantic_id.as_deref().unwrap_or(&element.id))}
                    style={|s| s.h_full().flex_row().justify_start().flex_shrink_0().font_family(CODE_FONT).text_size(px(12.)).line_height(px(20.)).whitespace_nowrap()}
                    on:click={ { let id = element.id.clone(); cx.listener(move |this, _, window, cx| this.select(&id, window, cx)) } }>
                    <span style={|s| s.text_color(rgb(colors.tag))}>{if row.closing { "</" } else { "<" }}</span>
                    <span style={|s| s.text_color(rgb(colors.tag))}>{kind.clone()}</span>
                    if !row.closing {
                        if let Some(id) = &element.semantic_id {
                            <span style={|s| s.text_color(rgb(colors.attribute))}>" id="</span>
                            <span style={|s| s.text_color(rgb(colors.value)).when(row.matched, |s| s.bg(rgb(if self.light { 0xffedb1 } else { 0x614c2b })))}>{serde_json::to_string(id).unwrap_or_default()}</span>
                        }
                        if row.collapsed {
                            <span style={|s| s.text_color(rgb(colors.tag))}> ">"</span>
                            <span style={|s| s.px(px(3.)).text_color(rgb(colors.muted))}>"…"</span>
                            <span style={|s| s.text_color(rgb(colors.tag))}>{format!("</{kind}>")}</span>
                        } else {
                            <span style={|s| s.text_color(rgb(colors.tag))}>{if row.branch { ">" } else { " />" }}</span>
                        }
                        if flex {
                            <span class="px-1" style={|s| s.ml(px(6.)).rounded(px(3.)).text_size(px(10.)).line_height(px(14.)).bg(rgb(colors.field)).text_color(rgb(colors.muted))}>"flex"</span>
                        }
                    } else { <span style={|s| s.text_color(rgb(colors.tag))}>">"</span> }
                </button>
            </div>
        }.into_any_element()
    }

    fn style_row(
        &self,
        row: &styles::Declaration<'_>,
        colors: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let expanded = self.expanded_styles.contains(&row.name);
        let complex = row.raw.is_object() || row.raw.is_array();

        rake::view! {
            <div id={format!("declaration-{}", row.name)} class="flex flex-col min-w-0">
                <div class="flex flex-row" style={|s| s.min_h(px(20.)).pl(px(8.)).pr(px(10.)).hover(|s| s.bg(rgb(colors.hover)))}>
                    if complex {
                        <button id={format!("expand-{}", row.name)} aria-label={format!("{} {} metadata", if expanded { "Collapse" } else { "Expand" }, row.name)} style={|s| s.w(px(16.)).h(px(20.)).flex_shrink_0()}
                            on:click={ { let name = row.name.clone(); cx.listener(move |this, _, _, cx| { if !this.expanded_styles.remove(&name) { this.expanded_styles.insert(name.clone()); } cx.notify(); }) } }>{icon(if expanded { Icon::Down } else { Icon::Right }, colors.muted)}</button>
                    } else { <span style={|s| s.w(px(16.)).flex_shrink_0()}/> }
                    <div class="flex flex-row min-w-0" style={|s| s.flex_wrap().gap_x(px(6.)).font_family(CODE_FONT).text_size(px(12.)).line_height(px(20.))}>
                        <span style={|s| s.text_color(rgb(colors.property))}>{format!("{}:", row.name)}</span>
                        <div class="flex flex-row items-center gap-1 min-w-0">
                            if let Some(color) = row.color {
                                <span aria-label="Authored color swatch" style={|s| s.w(px(11.)).h(px(11.)).flex_shrink_0().border_1().border_color(rgb(colors.muted)).bg(hsla(color.h, color.s, color.l, color.a))}/>
                            }
                            <span style={|s| s.text_color(rgb(colors.ink))}>{format!("{};", row.value)}</span>
                        </div>
                    </div>
                </div>
                if expanded {
                    <div class="px-3 py-2" style={|s| s.ml(px(24.)).max_h(px(180.)).overflow_scroll().bg(rgb(colors.bar)).font_family(CODE_FONT).text_size(px(11.)).line_height(px(17.)).text_color(rgb(colors.muted))}>{serde_json::to_string_pretty(row.raw).unwrap_or_default()}</div>
                }
            </div>
        }.into_any_element()
    }
}

impl Render for ElementsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        let narrow = width < 720.;
        let colors = Palette::new(self.light);
        let model = self.model.borrow();

        let capture = model
            .runtime
            .zip(model.tree.as_ref().map(|tree| tree.capture.clone()));

        if capture != self.capture {
            self.collapsed.clear();
            self.capture = capture;
        }

        if model.selected.is_none() {
            self.breadcrumbs.clear();
        }

        let target = model
            .instances
            .iter()
            .find(|instance| Some(instance.info.runtime_id) == model.runtime);

        let target_name = target.map_or_else(
            || "Select runtime".into(),
            |instance| {
                format!(
                    "{} · {}",
                    instance
                        .application_id
                        .rsplit('.')
                        .next()
                        .unwrap_or(&instance.application_id),
                    instance.info.platform
                )
            },
        );

        let error = self.config_error.as_ref().or(model.error.as_ref()).cloned();
        let selected = model.detail.as_ref();

        let elements = model
            .tree
            .as_ref()
            .map_or(&[][..], |tree| tree.elements.as_slice());

        let rows = tree::rows(elements, &model.filter, &self.collapsed);

        let style_rows = selected
            .and_then(|element| element.styles.as_ref())
            .map(styles::declarations)
            .unwrap_or_default();

        let style_count = style_rows
            .iter()
            .filter(|row| styles::matches(row, &self.style_query))
            .count();

        let stale = model.stale();

        let status = if model.disconnected {
            "Disconnected"
        } else if error.is_some() {
            "Connection error"
        } else if model.tree.is_some() {
            if model.paused { "Paused" } else { "Connected" }
        } else {
            "Connecting"
        };

        let capture_status = model.tree.as_ref().map_or_else(
            || "Waiting for capture".into(),
            |tree| {
                format!(
                    "{} boundaries · {} omitted · capture {}",
                    tree.elements.len(),
                    tree.omitted,
                    tree.capture
                )
            },
        );

        let current_tab = self.detail_tab;

        let tree_pane = rake::view! {
            <section id="tree-pane" class="flex flex-col min-h-0 min-w-0" aria-label="Element tree" style={|s| if narrow { s.h(px(260.)).w_full().flex_shrink_0() } else { s.w(px((width * self.split).clamp(180., (width - 260.).max(180.)))).flex_shrink_0() }}>
                if self.show_find {
                    <div class="flex flex-row items-center gap-2 px-2" style={|s| s.h(px(30.)).flex_shrink_0().border_b_1().border_color(rgb(colors.line))}>
                        {icon(Icon::Search, colors.muted)}
                        <div class="flex-1 min-w-0">{self.filter.clone()}</div>
                        <button id="close-find" aria-label="Close tree search" style={|s| s.w(px(20.)).h(px(24.))} on:click={cx.listener(Self::toggle_find)}>"×"</button>
                    </div>
                }
                <section id="element-tree" class="flex flex-col flex-1 min-h-0" style={|s| s.overflow_scroll().py(px(5.))}>
                    for row in &rows {
                        <div id={format!("tree-row-{}-{}", row.index, row.closing)} style={|s| s.h(px(20.)).flex_shrink_0()}>{self.tree_row(*row, &elements[row.index], colors, cx)}</div>
                    }
                    if elements.is_empty() {
                        <p class="px-3 py-4" style={|s| s.text_color(rgb(colors.muted))}>{if model.tree.is_some() { "No captured boundaries. Enable inspection in the target application." } else { "Select a running Rake application to inspect its views." }}</p>
                    } else if rows.is_empty() {
                        <p class="px-3 py-4" style={|s| s.text_color(rgb(colors.muted))}>"No matching elements."</p>
                    }
                </section>
                <div id="breadcrumbs" aria-label="Selected element ancestors" scroll={&self.breadcrumb_scroll} class="flex flex-row items-center min-w-0" style={|s| s.h(px(24.)).flex_shrink_0().overflow_x_scroll().border_t_1().border_color(rgb(colors.line)).bg(rgb(colors.bar)).font_family(CODE_FONT).text_size(px(10.))}>
                    for (id, label) in &self.breadcrumbs {
                        <div id={format!("crumb-{id}")} class="flex flex-row items-center" style={|s| s.h_full().flex_shrink_0()}>
                            if stale { <span class="px-2" style={|s| s.text_color(rgb(colors.muted)).whitespace_nowrap()}>{label.clone()}</span> }
                            else {
                                <button id={format!("select-crumb-{id}")} class="px-2" style={|s| s.h_full().whitespace_nowrap().bg(rgb(if model.selected.as_ref() == Some(id) { colors.field } else { colors.bar })).hover(|s| s.bg(rgb(colors.field)))}
                                    on:click={ { let id = id.clone(); cx.listener(move |this, _, window, cx| this.select(&id, window, cx)) } }>{label.clone()}</button>
                            }
                            {icon(Icon::Right, colors.muted)}
                        </div>
                    }
                    if self.breadcrumbs.is_empty() { <span class="px-2" style={|s| s.text_color(rgb(colors.muted))}>"Rake view tree"</span> }
                </div>
            </section>
        }.inspect("inspector.tree");

        let details = rake::view! {
            <section id="element-details" class="flex flex-col flex-1 min-w-0 min-h-0" aria-label="Element details">
                <div class="flex flex-row" style={|s| s.h(px(29.)).flex_shrink_0().bg(rgb(colors.bar)).border_b_1().border_color(rgb(colors.line))}>
                    for tab in ["Styles", "Layout", "Source"] {
                        <div id={format!("tab-container-{tab}")} style={|s| s.h_full()}>{self.detail_button(tab, colors, cx)}</div>
                    }
                </div>
                if current_tab == "Styles" {
                    <div class="flex flex-row items-center gap-2 px-2" style={|s| s.h(px(31.)).flex_shrink_0().border_b_1().border_color(rgb(colors.line))}>
                        {icon(Icon::Search, colors.muted)}
                        <div class="flex-1 min-w-0">{self.style_filter.clone()}</div>
                    </div>
                }
                <section id="detail-content" class="flex flex-col flex-1 min-h-0" style={|s| s.overflow_y_scroll()}>
                    if stale { <p class="px-3 py-1" style={|s| s.text_size(px(11.)).text_color(rgb(if self.light { 0x826113 } else { 0xfdd663 })).bg(rgb(if self.light { 0xfff6d9 } else { 0x3b3321 }))}>"Previous capture · details frozen"</p> }
                    if let Some(error) = &model.lookup_error { <p class="px-3 py-1" style={|s| s.text_size(px(11.)).text_color(rgb(colors.property))}>{error.clone()}</p> }
                    if let Some(element) = selected {
                        if current_tab == "Styles" {
                            <div id="authored-rule" class="flex flex-col" style={|s| s.border_b_1().border_color(rgb(colors.line)).pb(px(6.))}>
                                <div class="flex flex-row items-center gap-2 px-2" style={|s| s.min_h(px(30.)).font_family(CODE_FONT).text_size(px(11.))}>
                                    <span class="min-w-0" style={|s| s.text_color(rgb(colors.ink))}>{format!("{}{} {{", tree::tag(&element.kind), element.semantic_id.as_ref().map_or_else(String::new, |id| format!("[id={}]", serde_json::to_string(id).unwrap_or_default())))}</span>
                                    <span class="flex-1"/>
                                    <button id="rule-source" aria-label="Show inspection boundary source" aria-description="Source of the captured inspection boundary; declaration provenance is unavailable."
                                        style={|s| s.flex_shrink_0().text_color(rgb(colors.muted)).underline().hover(|s| s.text_color(rgb(colors.ink)))}
                                        on:click={cx.listener(|this, _, _, cx| { this.detail_tab = "Source"; cx.notify(); })}>{source_label(element)}</button>
                                </div>
                                for group in [styles::Group::Layout, styles::Group::Spacing, styles::Group::Appearance, styles::Group::Typography, styles::Group::Other] {
                                    <div id={format!("group-{}", group.label())} class="flex flex-col">
                                        if style_rows.iter().any(|row| row.group == group && styles::matches(row, &self.style_query)) {
                                            <span style={|s| s.pl(px(24.)).pt(px(3.)).text_color(rgb(colors.muted)).font_family(CODE_FONT).text_size(px(10.)).line_height(px(19.))}>{format!("/* {} */", group.label().to_lowercase())}</span>
                                            for row in style_rows.iter().filter(|row| row.group == group && styles::matches(row, &self.style_query)) {
                                                <div id={format!("property-row-{}", row.name)}>{self.style_row(row, colors, cx)}</div>
                                            }
                                        }
                                    </div>
                                }
                                if style_count == 0 { <p class="px-3 py-2" style={|s| s.text_color(rgb(colors.muted))}>{if style_rows.is_empty() { "No authored refinements captured." } else { "No matching properties." }}</p> }
                                <span class="px-2" style={|s| s.font_family(CODE_FONT)}>"}"</span>
                            </div>
                            <p class="px-3 py-2" style={|s| s.text_size(px(10.)).text_color(rgb(colors.muted))}>"Authored base refinements · read-only"</p>
                        } else {
                            <div class="px-3 py-2" style={|s| s.font_family(CODE_FONT).border_b_1().border_color(rgb(colors.line)).text_color(rgb(colors.tag))}>{format!("{}{}", tree::tag(&element.kind), element.semantic_id.as_ref().map_or_else(String::new, |id| format!("#{id}")))}</div>
                        }
                        if current_tab == "Layout" {
                            <p class="px-3 py-2" style={|s| s.text_color(rgb(colors.muted))}>"Bounds · window logical pixels"</p>
                            if let Some(layout) = element.layout {
                                <div class="m-4 p-4 flex flex-col gap-3 border" style={|s| s.border_color(rgb(colors.line)).items_center().font_family(CODE_FONT)}>
                                    <span style={|s| s.text_color(rgb(colors.muted))}>{format!("x: {:.1} · y: {:.1}", layout.x, layout.y)}</span>
                                    <div class="px-4 py-3 border" style={|s| s.border_color(rgb(colors.accent)).bg(rgb(colors.bar))}>{format!("{:.1} × {:.1}", layout.width, layout.height)}</div>
                                    <span style={|s| s.text_size(px(10.)).text_color(rgb(colors.muted))}>"Measured before clipping"</span>
                                </div>
                            } else { <p class="p-3">"Layout unavailable."</p> }
                        }
                        if current_tab == "Source" {
                            if let Some(source) = &element.source {
                                {property("Source file", source.file.clone(), colors)}
                                {property("Line", source.line.to_string(), colors)}
                                {property("Column", source.column.to_string(), colors)}
                            } else { <p class="p-3">"Source location unavailable."</p> }
                            {property("Rust type", element.kind.clone(), colors)}
                            {property("Capture ID", element.id.clone(), colors)}
                            {property("Semantic ID", element.semantic_id.clone().unwrap_or_else(|| "Unavailable".into()), colors)}
                            {property("Parent", element.parent.clone().unwrap_or_else(|| "Inspection root".into()), colors)}
                            {property("Entry", element.entry_id.map_or_else(|| "Workspace shell".into(), |id| id.to_string()), colors)}
                            <p class="px-3 py-2" style={|s| s.text_size(px(10.)).text_color(rgb(colors.muted))}>"Source identifies the inspection boundary."</p>
                        }
                        if element.metadata_truncated { <p class="px-3 py-2" style={|s| s.text_color(rgb(colors.property))}>"Some metadata exceeded capture limits."</p> }
                    } else { <p class="px-3 py-4" style={|s| s.text_color(rgb(colors.muted))}>"Select an element to inspect its authored styles, layout and source."</p> }
                </section>
            </section>
        }.inspect("inspector.details");

        rake::view! {
            <div id="inspector" class="size-full flex flex-col min-h-0" style={|s| s.relative().bg(rgb(colors.background)).text_color(rgb(colors.ink)).text_size(px(12.)).line_height(px(20.)).track_focus(&self.focus)}
                on:key-down={cx.listener(Self::key)}
                on:mouse-move={cx.listener(|this, event: &MouseMoveEvent, window, cx| { if this.dragging { if event.dragging() { this.split = (f32::from(event.position.x) / f32::from(window.viewport_size().width)).clamp(0.2, 0.8); cx.notify(); } else { this.dragging = false; } } })}
                on:mouse-up={cx.listener(|this, _, _, _| this.dragging = false)}>
                <div class="flex flex-row items-center" style={|s| s.h(px(32.)).flex_shrink_0().bg(rgb(colors.bar)).border_b_1().border_color(rgb(colors.line))}>
                    <div class="flex flex-row items-center gap-1 px-2" style={|s| s.text_color(rgb(colors.muted))}>{icon(Icon::Code, colors.muted)}<span>"Rake"</span></div>
                    <span style={|s| s.w(px(1.)).h(px(16.)).bg(rgb(colors.line))}/>
                    <div class="flex items-center px-3" style={|s| s.h_full().border_b_2().border_color(rgb(colors.accent)).text_color(rgb(colors.accent)).font_weight(FontWeight::SEMIBOLD)}>"Elements"</div>
                    <span class="flex-1"/>
                    <button id="targets" aria-label="Select target runtime" class="flex flex-row items-center gap-1 px-2" style={|s| s.h(px(26.)).text_color(rgb(colors.muted)).hover(|s| s.bg(rgb(colors.field)))}
                        on:click={cx.listener(|this, _, _, cx| { this.show_targets = !this.show_targets; cx.notify(); })}><span>{target_name}</span>{icon(Icon::Down, colors.muted)}</button>
                    <span style={|s| s.w(px(1.)).h(px(16.)).bg(rgb(colors.line))}/>
                    <button id="find" aria-label="Find in element tree" style={|s| s.w(px(30.)).h_full().hover(|s| s.bg(rgb(colors.field)))} on:click={cx.listener(Self::toggle_find)}>{icon(Icon::Search, colors.muted)}</button>
                    <button id="pause" aria-label={if model.paused { "Resume observation" } else { "Pause observation" }} style={|s| s.w(px(30.)).h_full().hover(|s| s.bg(rgb(colors.field)))} on:click={cx.listener(|this, _, _, cx| { let mut model = this.model.borrow_mut(); model.paused = !model.paused; cx.notify(); })}>{icon(if model.paused { Icon::Play } else { Icon::Pause }, if model.paused { colors.accent } else { colors.muted })}</button>
                    <button id="refresh" aria-label="Refresh capture" style={|s| s.w(px(30.)).h_full().hover(|s| s.bg(rgb(colors.field)))} on:click={cx.listener(|this, _, _, cx| { this.model.borrow_mut().refresh = true; cx.notify(); })}>{icon(Icon::Refresh, colors.muted)}</button>
                    <button id="theme" aria-label="Toggle theme" style={|s| s.w(px(30.)).h_full().hover(|s| s.bg(rgb(colors.field)))} on:click={cx.listener(|this, _, _, cx| { this.light = !this.light; this.filter.update(cx, |input, cx| input.set_light(this.light, cx)); this.style_filter.update(cx, |input, cx| input.set_light(this.light, cx)); cx.notify(); })}>{icon(Icon::Theme, colors.muted)}</button>
                </div>
                if let Some(error) = error { <p class="px-3 py-1" style={|s| s.text_color(rgb(colors.property)).border_b_1().border_color(rgb(colors.line))}>{error}</p> }
                <div class="flex flex-1 min-h-0 min-w-0" style={|s| if narrow { s.flex_col() } else { s.flex_row() }}>
                    {tree_pane}
                    if !narrow {
                        <div id="divider" tabindex={0} aria-label="Resize inspection panes" style={|s| s.w(px(4.)).h_full().flex_shrink_0().border_l_1().border_color(rgb(colors.line)).cursor(CursorStyle::ResizeColumn).track_focus(&self.divider).hover(|s| s.bg(rgb(colors.field)))}
                            on:mouse-down={cx.listener(|this, _, window, cx| { this.dragging = true; window.focus(&this.divider, cx); })}
                            on:key-down={cx.listener(|this, event: &KeyDownEvent, _, cx| { match event.keystroke.key.as_str() { "left" => this.split = (this.split - 0.025).max(0.2), "right" => this.split = (this.split + 0.025).min(0.8), "home" => this.split = 0.2, "end" => this.split = 0.8, _ => return } cx.stop_propagation(); cx.notify(); })}/>
                    }
                    {details}
                </div>
                if self.show_coverage {
                    <section class="px-3 py-2" style={|s| s.flex_shrink_0().border_t_1().border_color(rgb(colors.line)).bg(rgb(colors.bar)).text_size(px(11.)).text_color(rgb(colors.muted))} aria-label="Inspection coverage">
                        <p>{model.tree.as_ref().map_or_else(|| "Waiting for capture coverage.".into(), |tree| tree.coverage.clone())}</p>
                        <p>"Tree id attributes show semantic IDs. Source links identify inspection boundaries."</p>
                    </section>
                }
                <div class="flex flex-row items-center gap-2 px-2" style={|s| s.h(px(23.)).flex_shrink_0().bg(rgb(colors.bar)).border_t_1().border_color(rgb(colors.line)).text_size(px(10.)).text_color(rgb(colors.muted))}>
                    <button id="coverage" aria-label="Show capture and coverage details" class="flex flex-row items-center gap-1" style={|s| s.h_full().hover(|s| s.text_color(rgb(colors.ink)))} on:click={cx.listener(|this, _, _, cx| { this.show_coverage = !this.show_coverage; cx.notify(); })}>{icon(if self.show_coverage { Icon::Down } else { Icon::Right }, colors.muted)}<span>{capture_status}</span></button>
                    <span class="flex-1"/>
                    <span style={|s| s.w(px(6.)).h(px(6.)).rounded_full().bg(rgb(if model.disconnected { colors.muted } else if self.config_error.is_some() || model.error.is_some() { colors.property } else { 0x6d9668 }))}/>
                    <span>{status}</span>
                    <button id="disconnect" style={|s| s.h_full().px(px(5.)).hover(|s| s.text_color(rgb(colors.ink)))} on:click={cx.listener(|this, _, _, cx| { let mut model = this.model.borrow_mut(); model.disconnected = !model.disconnected; let runtime = model.runtime; model.select_runtime(runtime); cx.notify(); })}>{if model.disconnected { "Reconnect" } else { "Disconnect" }}</button>
                </div>
                if self.show_targets {
                    <section id="target-menu" class="flex flex-col p-1 border" style={|s| s.absolute().top(px(32.)).right(px(6.)).w(px(490.)).max_w_full().max_h(px(220.)).overflow_y_scroll().bg(rgb(colors.background)).border_color(rgb(colors.line)).shadow_lg()}>
                        for instance in &model.instances {
                            <button id={format!("runtime-{}", instance.info.runtime_id)} class="px-3 py-2" style={|s| s.justify_start().text_size(px(11.)).hover(|s| s.bg(rgb(colors.hover)))} on:click={ { let runtime = instance.info.runtime_id; cx.listener(move |this, _, _, cx| this.select_runtime(runtime, cx)) } }>{format!("{} · {}\n{}", instance.application_id, instance.info.platform, instance.info.runtime_id)}</button>
                        }
                        if model.instances.is_empty() { <p class="p-3">"No runtimes advertised by this broker."</p> }
                    </section>
                }
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
    use super::{Palette, property};
    use rake::gpui::{Element, IntoElement};

    #[test]
    fn source_properties_have_distinct_stable_sibling_identities() {
        let mut ids = std::collections::HashSet::new();

        for label in [
            "Source file",
            "Line",
            "Column",
            "Rust type",
            "Capture ID",
            "Semantic ID",
            "Parent",
            "Entry",
        ] {
            let element = property(label, "value", Palette::new(true)).into_element();

            assert!(
                ids.insert(Element::id(&element).expect("property identity")),
                "duplicate {label}"
            );
        }
    }
}
