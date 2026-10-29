//! Macro-authored Elements screen; remote work is owned by its Rake entry activity.

use crate::appearance::{CODE_FONT, Icon, Palette, control, icon};
use crate::input::FilterInput;
use crate::{Connection, ConnectionConfig, Inspector, Update};
use crate::{styles, tree};
use rake::control::{Command, Query, Reply};
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
    tree_scroll: ScrollHandle,
    detail_scrolls: [ScrollHandle; 3],
    tree_focus: FocusHandle,
    _focus_observers: [Subscription; 3],
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

        rake::theme::install(cx, Palette::new(true));
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
                    window,
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
                    window,
                    cx,
                )
            });

            let tree_focus = cx.focus_handle();

            let focus_observers = [
                cx.on_focus(&tree_focus, window, |_, _, cx| cx.notify()),
                cx.on_blur(&tree_focus, window, |_, _, cx| cx.notify()),
                cx.observe_window_activation(window, |_, _, cx| cx.notify()),
            ];

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
                tree_scroll: ScrollHandle::new(),
                detail_scrolls: std::array::from_fn(|_| ScrollHandle::new()),
                tree_focus,
                _focus_observers: focus_observers,
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

        let focus = view.read(cx).tree_focus.clone();

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

                    let mut ticks = 0u64;
                    let mut controlled = None;
                    let mut highlighting = false;

                    loop {
                        let (epoch, disconnected) = {
                            let model = observed.borrow();

                            (model.epoch, model.disconnected)
                        };

                        let (target, pending, picking, hover, supported) = {
                            let mut model = observed.borrow_mut();

                            let supported = model.instances.iter().any(|i| {
                                Some(i.info.runtime_id) == model.runtime
                                    && i.info.element_interaction
                            });

                            (
                                model.runtime,
                                std::mem::take(&mut model.mode_pending),
                                model.picking,
                                model.hover.clone(),
                                supported,
                            )
                        };

                        let target = target.filter(|_| !disconnected && supported);

                        if controlled != target {
                            if let Some(old) = controlled {
                                let _ = connection
                                    .control(
                                        old,
                                        Command::InspectMode {
                                            enabled: false,
                                            lease_ms: 0,
                                        },
                                        run.cancellation(),
                                    )
                                    .await;

                                let _ = connection
                                    .control(
                                        old,
                                        Command::HighlightElement {
                                            id: None,
                                            generation: String::new(),
                                        },
                                        run.cancellation(),
                                    )
                                    .await;
                            }

                            controlled = target;
                            highlighting = false;
                        }

                        if let Some(runtime) = target {
                            let (navigation, source) = {
                                let mut model = observed.borrow_mut();
                                (model.navigation_pending.take(), model.source_pending.take())
                            };
                            if let Some(command) = navigation {
                                let result = connection
                                    .control(runtime, command, run.cancellation())
                                    .await;
                                run.publish(Update::Navigation(epoch, runtime, result))?;
                            }
                            if let Some(id) = source {
                                let result = connection
                                    .open_source(runtime, &id, run.cancellation())
                                    .await;
                                run.publish(Update::Navigation(epoch, runtime, result))?;
                            }
                            if let Some(enabled) = pending {
                                let result = connection
                                    .control(
                                        runtime,
                                        Command::InspectMode {
                                            enabled,
                                            lease_ms: 1000,
                                        },
                                        run.cancellation(),
                                    )
                                    .await;

                                if let Err(error) = result {
                                    run.publish(Update::Pick(epoch, runtime, Err(error)))?;
                                }
                            }

                            if picking {
                                let result = connection
                                    .query(runtime, Query::InspectPick {}, run.cancellation())
                                    .await
                                    .and_then(|reply| match reply {
                                        Reply::InspectPick(pick) => Ok(pick),
                                        _ => Err("Unexpected picker reply".into()),
                                    });

                                if result.as_ref().is_ok_and(|pick| pick.mode) {
                                    let _ = connection
                                        .control(
                                            runtime,
                                            Command::InspectMode {
                                                enabled: true,
                                                lease_ms: 1000,
                                            },
                                            run.cancellation(),
                                        )
                                        .await;
                                }

                                run.publish(Update::Pick(epoch, runtime, result))?;
                            } else if let Some((id, generation)) = hover {
                                let _ = connection
                                    .control(
                                        runtime,
                                        Command::HighlightElement {
                                            id: Some(id),
                                            generation,
                                        },
                                        run.cancellation(),
                                    )
                                    .await;

                                highlighting = true;
                            } else if highlighting {
                                let _ = connection
                                    .control(
                                        runtime,
                                        Command::HighlightElement {
                                            id: None,
                                            generation: String::new(),
                                        },
                                        run.cancellation(),
                                    )
                                    .await;

                                highlighting = false;
                            }
                        }

                        if !disconnected && ticks.is_multiple_of(5) {
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

                                    run.publish(Update::Element(
                                        epoch,
                                        runtime,
                                        id,
                                        Box::new(result),
                                    ))?;
                                }

                                if refresh {
                                    let result = connection.tree(runtime, run.cancellation()).await;

                                    run.publish(Update::Tree(epoch, runtime, result))?;
                                }
                            }
                        }

                        ticks = ticks.wrapping_add(1);
                        rake::tasks::sleep(Duration::from_millis(100)).await;
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
        <div id={format!("property-{label}")} class="flex flex-row gap-3 px-3 py-1" style={|s| s.border_b_1().border_color(colors.line)}>
            <span style={|s| s.w(px(86.)).flex_shrink_0().text_color(colors.muted)}>{label}</span>
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
    fn navigate(&self, action: rake::inspection::NavigationAction) {
        let mut model = self.model.borrow_mut();
        if let (Some(id), Some(tree)) = (&model.selected, &model.tree)
            && tree.elements.iter().any(|element| &element.id == id)
        {
            model.navigation_pending = Some(Command::NavigateElement {
                id: id.clone(),
                generation: tree.capture.clone(),
                action,
            });
        }
    }

    fn detail_button(&self, tab: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <button id={format!("detail-{tab}")} class={if self.detail_tab == tab { "px-3 h-full border-accent hover:bg-hover text-accent border-b-[2px]" } else { "px-3 h-full border-accent hover:bg-hover text-muted border-b-0" }} style={control}
                on:click={cx.listener(move |this, _, _, cx| { this.detail_tab = tab; cx.notify(); })}>{tab}</button>
        }.inspect(format!("inspector.tab.{tab}"))
    }

    fn select(&mut self, id: &str, reveal: bool, window: &mut Window, cx: &mut Context<Self>) {
        {
            let mut model = self.model.borrow_mut();

            let Some(capture) = &model.tree else {
                return;
            };

            let path = tree::ancestors(&capture.elements, id);

            if path.is_empty() {
                return;
            }

            self.breadcrumbs = path
                .iter()
                .map(|element| {
                    let label = format!(
                        "{}{}",
                        element.display_name().to_owned(),
                        element
                            .semantic_id
                            .as_ref()
                            .map_or_else(String::new, |id| format!("#{id}"))
                    );

                    (element.id.clone(), label)
                })
                .collect();
            model.select_element(id);
        }
        self.breadcrumb_scroll
            .scroll_to_item(self.breadcrumbs.len().saturating_sub(1));
        self.expanded_styles.clear();

        if reveal {
            self.reveal_selection(window, cx);
        }

        window.focus(&self.tree_focus, cx);
        cx.notify();
    }

    fn reveal_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (id, path, clear_filter) = {
            let model = self.model.borrow();

            let Some(capture) = &model.tree else {
                return;
            };

            let Some(id) = &model.selected else {
                return;
            };

            let path = tree::ancestors(&capture.elements, id);

            if path.is_empty() {
                return;
            }

            let uncollapsed = tree::rows(&capture.elements, &model.filter, &BTreeSet::new());

            let clear_filter = !uncollapsed
                .iter()
                .any(|row| !row.closing && capture.elements[row.index].id == *id);

            (
                id.clone(),
                path.iter()
                    .map(|element| element.id.clone())
                    .collect::<Vec<_>>(),
                clear_filter,
            )
        };

        for ancestor in path.iter().filter(|ancestor| **ancestor != id) {
            self.collapsed.remove(ancestor);
        }

        if clear_filter {
            self.filter.update(cx, |input, cx| input.reset(cx));
            self.model.borrow_mut().filter.clear();
        }

        let model = self.model.borrow();

        if let Some(capture) = &model.tree {
            let rows = tree::rows(&capture.elements, &model.filter, &self.collapsed);

            if let Some(line) = rows
                .iter()
                .position(|row| !row.closing && capture.elements[row.index].id == id)
            {
                let offset = self.tree_scroll.offset();
                let height = f32::from(self.tree_scroll.bounds().size.height);
                let y = tree::reveal_offset(f32::from(offset.y), height, line, rows.len());

                self.tree_scroll.set_offset(point(offset.x, px(y)));
            }
        }

        window.focus(&self.tree_focus, cx);
        cx.notify();
    }

    fn toggle_branch(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.collapsed.remove(id) {
            self.collapsed.insert(id.to_owned());
            let hides_selected = {
                let model = self.model.borrow();

                model
                    .tree
                    .as_ref()
                    .zip(model.selected.as_deref())
                    .is_some_and(|(capture, selected)| {
                        selected != id
                            && tree::ancestors(&capture.elements, selected)
                                .iter()
                                .any(|ancestor| ancestor.id == id)
                    })
            };

            if hides_selected {
                self.select(id, true, window, cx);
            }
        }

        window.focus(&self.tree_focus, cx);
        cx.notify();
    }

    fn tree_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tree_focus.is_focused(window) || event.keystroke.modifiers.modified() {
            return;
        }

        let key = match event.keystroke.key.as_str() {
            "up" => tree::Key::Up,
            "down" => tree::Key::Down,
            "left" => tree::Key::Left,
            "right" => tree::Key::Right,
            "home" => tree::Key::Home,
            "end" => tree::Key::End,
            "pageup" => tree::Key::PageUp,
            "pagedown" => tree::Key::PageDown,
            "enter" | "space" => tree::Key::Toggle,
            _ => return,
        };

        let change = {
            let model = self.model.borrow();

            model.tree.as_ref().and_then(|capture| {
                let rows = tree::rows(&capture.elements, &model.filter, &self.collapsed);

                let page_lines = (f32::from(self.tree_scroll.bounds().size.height)
                    / tree::ROW_HEIGHT)
                    .floor()
                    .max(1.) as usize;

                tree::navigate(
                    &capture.elements,
                    &rows,
                    model.selected.as_deref(),
                    key,
                    page_lines,
                )
                .map(|change| {
                    let index = match change {
                        tree::Change::Select(i)
                        | tree::Change::Expand(i)
                        | tree::Change::Collapse(i) => i,
                    };

                    (change, capture.elements[index].id.clone())
                })
            })
        };

        if let Some((change, id)) = change {
            match change {
                tree::Change::Select(_) => self.select(&id, true, window, cx),
                tree::Change::Expand(_) | tree::Change::Collapse(_) => {
                    self.toggle_branch(&id, window, cx);
                    self.reveal_selection(window, cx);
                }
            }
        }

        window.prevent_default();
        cx.stop_propagation();
    }

    fn select_runtime(&mut self, runtime: RuntimeId, cx: &mut Context<Self>) {
        let mut model = self.model.borrow_mut();

        model.disconnected = false;
        model.select_runtime(Some(runtime));
        self.show_targets = false;

        for scroll in [
            &self.tree_scroll,
            &self.breadcrumb_scroll,
            &self.detail_scrolls[0],
            &self.detail_scrolls[1],
            &self.detail_scrolls[2],
        ] {
            scroll.set_offset(Point::default());
        }

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
            window.focus(&self.tree_focus, cx);
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
        tree_active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = self.model.borrow().selected.as_ref() == Some(&element.id);
        let opening_selected = selected && !row.closing;

        let background = if selected {
            if opening_selected && tree_active {
                colors.selection
            } else {
                colors.inactive_selection
            }
        } else {
            colors.background
        };

        let hover_id = element.id.clone();
        let kind = element.display_name().to_owned();

        let flex = element
            .styles
            .as_ref()
            .is_some_and(|styles| styles["display"] == "Flex");

        rake::view! {
            <div id={format!("line-{}-{}", row.index, row.closing)} aria-label={format!("Inspect {}{}", element.semantic_id.as_deref().unwrap_or(&element.id), if row.closing { " (closing tag)" } else { "" })}
                class="flex flex-row items-center min-w-0 shrink-0 pl-[4px] border-l-2"
                style={|s| s.h(px(tree::ROW_HEIGHT)).min_w_full().border_color(if opening_selected && tree_active { colors.focus } else { rgba(0) }).bg(background).hover(|s| s.bg(if selected { background } else { colors.hover })).when(!row.closing, |s| s.role(gpui::accesskit::Role::TreeItem).aria_level(row.depth + 1).aria_selected(opening_selected).when(row.branch, |s| s.aria_expanded(!row.collapsed)).when(opening_selected, |s| s.aria_active_descendant()))}
                on:hover={cx.listener(move |this, hovered: &bool, _, _| {
                    let mut model=this.model.borrow_mut();
                    if *hovered {
                        if let Some(tree)=&model.tree {model.hover=Some((hover_id.clone(),tree.capture.clone()));}
                    } else if model.hover.as_ref().is_some_and(|(id,_)|id==&hover_id) {model.hover=None;}
                })}
                on:click={ { let id = element.id.clone(); cx.listener(move |this, _, window, cx| this.select(&id, false, window, cx)) } }>
                for depth in 0..row.depth.min(16) {
                    <span id={format!("indent-{depth}")} class="w-[12px] h-full shrink-0 border-l-1 border-guide"/>
                }
                if row.branch && !row.closing {
                    <button id={format!("disclose-{}", row.index)} tabindex={-1} aria-label={format!("{} {}", if row.collapsed { "Expand" } else { "Collapse" }, element.semantic_id.as_deref().unwrap_or(&element.id))}
                        class="w-[14px] h-full shrink-0" style={control}
                        on:click={ { let id = element.id.clone(); cx.listener(move |this, _, window, cx| { this.toggle_branch(&id, window, cx); cx.stop_propagation(); }) } }>
                        {icon(if row.collapsed { Icon::Right } else { Icon::Down }, colors.muted)}
                    </button>
                } else { <span class="w-[14px] shrink-0"/> }
                <span class="flex flex-row h-full justify-start shrink-0 font-code text-[12px] whitespace-nowrap" style={|s| s.line_height(px(tree::ROW_HEIGHT))}>
                    <span class="text-tag">{if row.closing { "</" } else { "<" }}</span>
                    <span class="text-tag">{kind.clone()}</span>
                    if !row.closing {
                        if let Some(id) = &element.semantic_id {
                            <span class="text-attribute">" id="</span>
                            <span class="text-value" class:bg-matched={row.matched}>{serde_json::to_string(id).unwrap_or_default()}</span>
                        }
                        if row.collapsed {
                            <span class="text-tag"> ">"</span>
                            <span class="px-[3px] text-muted">"…"</span>
                            <span class="text-tag">{format!("</{kind}>")}</span>
                        } else {
                            <span class="text-tag">{if row.branch { ">" } else { " />" }}</span>
                        }
                        if flex {
                            <span class="px-1 ml-[6px] rounded-[3px] text-[10px] leading-[14px] bg-field text-muted">"flex"</span>
                        }
                    } else { <span class="text-tag">">"</span> }
                </span>
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
                <div class="flex flex-row min-h-[20px] pl-[8px] pr-[10px] hover:bg-hover">
                    if complex {
                        <button id={format!("expand-{}", row.name)} aria-label={format!("{} {} metadata", if expanded { "Collapse" } else { "Expand" }, row.name)} class="w-[16px] h-[20px] shrink-0" style={control}
                            on:click={ { let name = row.name.clone(); cx.listener(move |this, _, _, cx| { if !this.expanded_styles.remove(&name) { this.expanded_styles.insert(name.clone()); } cx.notify(); }) } }>{icon(if expanded { Icon::Down } else { Icon::Right }, colors.muted)}</button>
                    } else { <span class="w-[16px] shrink-0"/> }
                    <div class="flex flex-row min-w-0 flex-wrap gap-x-[6px] font-code text-[12px] leading-[20px]">
                        <span class="text-property">{format!("{}:", row.name)}</span>
                        <div class="flex flex-row items-center gap-1 min-w-0">
                            if let Some(color) = row.color {
                                <span aria-label="Authored color swatch" class="w-[11px] h-[11px] shrink-0 border-1 border-muted" style={|s| s.bg(hsla(color.h, color.s, color.l, color.a))}/>
                            }
                            <span class="text-ink">{format!("{};", row.value)}</span>
                        </div>
                    </div>
                </div>
                if expanded {
                    <div class="px-3 py-2 ml-[24px] max-h-[180px] overflow-scroll bg-bar font-code text-[11px] leading-[17px] text-muted">{serde_json::to_string_pretty(row.raw).unwrap_or_default()}</div>
                }
            </div>
        }.into_any_element()
    }
}

impl Render for ElementsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = f32::from(window.viewport_size().width);
        let narrow = width < 720.;
        let colors = *rake::theme::current::<Palette>();
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
        let tree_active = self.tree_focus.is_focused(window) && window.is_window_active();

        let can_reveal = model
            .selected
            .as_ref()
            .is_some_and(|id| elements.iter().any(|element| element.id == *id));

        let style_rows = selected
            .and_then(|element| {
                if self.detail_tab == "Computed" {
                    element
                        .resolved
                        .as_ref()
                        .map(|resolved| &resolved.properties)
                } else {
                    element.styles.as_ref()
                }
            })
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

        let detail_scroll = self.detail_scrolls[match current_tab {
            "Styles" => 0,
            "Layout" => 1,
            _ => 2,
        }]
        .clone();

        let tree_pane = rake::view! {
            <section id="tree-pane" class="flex flex-col min-h-0 min-w-0" aria-label="Element tree" style={|s| if narrow { s.h(px(260.)).w_full().flex_shrink_0() } else { s.w(px((width * self.split).clamp(180., (width - 260.).max(180.)))).flex_shrink_0() }}>
                if self.show_find {
                    <div class="flex flex-row items-center gap-2 px-2 h-[30px] shrink-0 border-b-1 border-line">
                        {icon(Icon::Search, colors.muted)}
                        <div class="flex-1 min-w-0">{self.filter.clone()}</div>
                        <button id="close-find" aria-label="Close tree search" class="w-[20px] h-[24px]" style={control} on:click={cx.listener(Self::toggle_find)}>"×"</button>
                    </div>
                }
                <section id="element-tree" role={gpui::accesskit::Role::Tree} aria-label="Element tree" aria-keyshortcuts="ArrowUp ArrowDown ArrowLeft ArrowRight Home End PageUp PageDown Enter Space"
                    tabindex={0} focus={&self.tree_focus} scroll={&self.tree_scroll} class="flex flex-col flex-1 min-h-0 overflow-scroll py-[5px] border-1 border-[#00000000]"
                    class:border-focus={tree_active}
                    on:key-down={cx.listener(Self::tree_key)} on:mouse-down={cx.listener(|this, _, window, cx| { window.focus(&this.tree_focus, cx); window.prevent_default(); })}>
                    for row in &rows {
                        <div id={format!("tree-row-{}-{}-{}-{}", model.epoch, model.tree.as_ref().map_or("0", |capture| capture.capture.as_str()), row.index, row.closing)} class="shrink-0" style={|s| s.h(px(tree::ROW_HEIGHT))}>{self.tree_row(*row, &elements[row.index], colors, tree_active, cx)}</div>
                    }
                    if elements.is_empty() {
                        <p class="px-3 py-4 text-muted">{if model.tree.is_some() { "No captured boundaries. Enable inspection in the target application." } else { "Select a running Rake application to inspect its views." }}</p>
                    } else if rows.is_empty() {
                        <p class="px-3 py-4 text-muted">"No matching elements."</p>
                    }
                </section>
                <div id="breadcrumbs" aria-label="Selected element ancestors" scroll={&self.breadcrumb_scroll} class="flex flex-row items-center min-w-0 h-[24px] shrink-0 overflow-x-scroll border-t-1 border-line bg-bar font-code text-[10px]">
                    for (id, label) in &self.breadcrumbs {
                        <div id={format!("crumb-{id}")} class="flex flex-row items-center h-full shrink-0">
                            if stale { <span class="px-2 text-muted whitespace-nowrap">{label.clone()}</span> }
                            else {
                                <button id={format!("select-crumb-{id}")} class={if model.selected.as_ref() == Some(id) { "px-2 h-full whitespace-nowrap hover:bg-field bg-field" } else { "px-2 h-full whitespace-nowrap hover:bg-field bg-bar" }} style={control}
                                    on:click={ { let id = id.clone(); cx.listener(move |this, _, window, cx| this.select(&id, true, window, cx)) } }>{label.clone()}</button>
                            }
                            {icon(Icon::Right, colors.muted)}
                        </div>
                    }
                    if self.breadcrumbs.is_empty() { <span class="px-2 text-muted">"Rake view tree"</span> }
                </div>
            </section>
        }.inspect("inspector.tree");

        let details = rake::view! {
            <section id="element-details" class="flex flex-col flex-1 min-w-0 min-h-0" aria-label="Element details">
                <div class="flex flex-row h-[29px] shrink-0 bg-bar border-b-1 border-line">
                    for tab in ["Styles", "Computed", "Layout", "Source"] {
                        <div id={format!("tab-container-{tab}")} class="h-full">{self.detail_button(tab, cx)}</div>
                    }
                </div>
                <div class="flex flex-row flex-wrap gap-1 px-2 py-1 border-b-1 border-line">
                    <button id="scroll-target" disabled={!can_reveal || !selected.is_some_and(|e| e.navigation.scroll_into_view)} class="px-2 py-1 hover:bg-field" style={control}
                        on:click={cx.listener(|this, _, _, cx| { this.navigate(rake::inspection::NavigationAction::ScrollIntoView); cx.notify(); })}>"Scroll into view"</button>
                    <button id="focus-target" disabled={!can_reveal || !selected.is_some_and(|e| e.navigation.focus)} class="px-2 py-1 hover:bg-field" style={control}
                        on:click={cx.listener(|this, _, _, cx| { this.navigate(rake::inspection::NavigationAction::Focus); cx.notify(); })}>"Focus element"</button>
                    <button id="open-rust-source" disabled={!can_reveal || selected.is_none_or(|e| e.source.is_none())} class="px-2 py-1 hover:bg-field" style={control}
                        on:click={cx.listener(|this, _, _, cx| { let mut model=this.model.borrow_mut(); model.source_pending=model.selected.clone(); cx.notify(); })}>"Open Rust source"</button>
                </div>
                if current_tab == "Styles" || current_tab == "Computed" {
                    <div class="flex flex-row items-center gap-2 px-2 h-[31px] shrink-0 border-b-1 border-line">
                        {icon(Icon::Search, colors.muted)}
                        <div class="flex-1 min-w-0">{self.style_filter.clone()}</div>
                    </div>
                }
                <section id={format!("detail-content-{current_tab}")} scroll={&detail_scroll} class="flex flex-col flex-1 min-h-0 overflow-y-scroll">
                    if stale { <p class="px-3 py-1 text-[11px] text-warning bg-warning-surface">"Previous capture · details frozen"</p> }
                    if let Some(error) = &model.lookup_error { <p class="px-3 py-1 text-[11px] text-property">{error.clone()}</p> }
                    if let Some(element) = selected {
                        if current_tab == "Styles" || current_tab == "Computed" {
                            <div id="authored-rule" class="flex flex-col border-b-1 border-line pb-[6px]">
                                <div class="flex flex-row items-center gap-2 px-2 min-h-[30px] font-code text-[11px]">
                                    <span class="min-w-0 text-ink">{format!("{}{} {{", element.display_name().to_owned(), element.semantic_id.as_ref().map_or_else(String::new, |id| format!("[id={}]", serde_json::to_string(id).unwrap_or_default())))}</span>
                                    <span class="flex-1"/>
                                    <button id="rule-source" aria-label="Show inspection boundary source" aria-description="Source of the captured inspection boundary; individual declaration source locations are unavailable."
                                        class="shrink-0 text-muted hover:text-ink" style={|s| control(s).underline()}
                                        on:click={cx.listener(|this, _, _, cx| { this.detail_tab = "Source"; cx.notify(); })}>{source_label(element)}</button>
                                </div>
                                for group in [styles::Group::Layout, styles::Group::Spacing, styles::Group::Appearance, styles::Group::Typography, styles::Group::Other] {
                                    <div id={format!("group-{}", group.label())} class="flex flex-col">
                                        if style_rows.iter().any(|row| row.group == group && styles::matches(row, &self.style_query)) {
                                            <span class="pl-[24px] pt-[3px] text-muted font-code text-[10px] leading-[19px]">{format!("/* {} */", group.label().to_lowercase())}</span>
                                            for row in style_rows.iter().filter(|row| row.group == group && styles::matches(row, &self.style_query)) {
                                                <div id={format!("property-row-{}", row.name)}>
                                                    {self.style_row(row, colors, cx)}
                                                    if current_tab == "Computed" {
                                                        if let Some(layer) = element.resolved.as_ref().and_then(|resolved| resolved.layers.iter().find(|layer| styles::declarations(&layer.properties).iter().any(|property| property.name == row.name))) {
                                                            <span class="pl-[24px] text-[10px] text-muted">{layer.origin.label()}</span>
                                                        }
                                                    }
                                                </div>
                                            }
                                        }
                                    </div>
                                }
                                if style_count == 0 { <p class="px-3 py-2 text-muted">{if style_rows.is_empty() { if current_tab == "Computed" { "Resolved styles unavailable for this boundary." } else { "No authored refinements captured." } } else { "No matching properties." }}</p> }
                                <span class="px-2 font-code">"}"</span>
                            </div>
                            <p class="px-3 py-2 text-[10px] text-muted">{if current_tab == "Computed" { if element.resolved.as_ref().is_some_and(|resolved| resolved.hovered) { "Resolved GPUI styles · hovered · read-only" } else { "Resolved GPUI styles · read-only" } } else { "Authored base refinements · read-only" }}</p>
                            if current_tab == "Computed" {
                                <p class="px-3 py-2 text-[10px] text-muted">"Origins describe observed values. Interaction rules with equal values and individual Rust declaration locations are unavailable. Inherited text may pass through uninspected ancestors."</p>
                            }
                        } else {
                            <div class="px-3 py-2 font-code border-b-1 border-line text-tag">{format!("{}{}", element.display_name().to_owned(), element.semantic_id.as_ref().map_or_else(String::new, |id| format!("#{id}")))}</div>
                        }
                        if current_tab == "Layout" {
                            <p class="px-3 py-2 text-muted">"Bounds · window logical pixels"</p>
                            if let Some(layout) = element.layout {
                                <div class="m-4 p-4 flex flex-col gap-3 border border-line items-center font-code">
                                    <span class="text-muted">{format!("x: {:.1} · y: {:.1}", layout.x, layout.y)}</span>
                                    <div class="px-4 py-3 border border-accent bg-bar">{format!("{:.1} × {:.1}", layout.width, layout.height)}</div>
                                    <span class="text-[10px] text-muted">"Measured before clipping"</span>
                                </div>
                            } else { <p class="p-3">"Layout unavailable."</p> }
                            if let Some(boxes) = &element.box_model {
                                for (name, bounds) in [("Margin box", boxes.margin), ("Border box", Some(boxes.border)), ("Padding box", boxes.padding), ("Content box", boxes.content), ("Visible intersection", element.content_mask)] {
                                    <div id={name}>{property(name, bounds.map_or_else(|| "Unavailable".into(), |bounds| format!("x {:.1}, y {:.1} · {:.1} × {:.1}", bounds.x, bounds.y, bounds.width, bounds.height)), colors)}</div>
                                }
                                for reason in &boxes.unavailable {
                                    <p id={reason.clone()} class="px-3 py-1 text-[10px] text-muted">{reason.clone()}</p>
                                }
                            } else {
                                <p class="px-3 py-2 text-muted">"Resolved box geometry unavailable for this boundary/runtime."</p>
                            }
                        }
                        if current_tab == "Source" {
                            if let Some(source) = &element.source {
                                {property("Source file", source.file.clone(), colors)}
                                {property("Line", source.line.to_string(), colors)}
                                {property("Column", source.column.to_string(), colors)}
                            } else { <p class="p-3">"Source location unavailable."</p> }
                            {property("Rust type", element.kind.clone(), colors)}
                            if let Some(view) = &element.view {
                                {property("Authored tag", view.tag.clone(), colors)}
                                for (index, style) in view.style_sources.iter().enumerate() {
                                    <div id={format!("style-source-{index}")} class="px-3 py-2 border-b-1 border-line font-code text-[11px]">
                                        <p>{format!("{} · {}", style.scope, style.attribute)}</p>
                                        <p class="text-muted">{style.expression.clone()}</p>
                                        <p class="text-muted">{format!("{}:{}:{}", style.source.file, style.source.line, style.source.column)}</p>
                                    </div>
                                }
                                <p class="px-3 py-2 text-[10px] text-muted">"Style sources list authored declarations, including conditional branches. They do not imply that a branch is active or locate methods inside opaque style closures."</p>
                            }
                            {property("Capture ID", element.id.clone(), colors)}
                            {property("Semantic ID", element.semantic_id.clone().unwrap_or_else(|| "Unavailable".into()), colors)}
                            {property("Parent", element.parent.clone().unwrap_or_else(|| "Inspection root".into()), colors)}
                            {property("Entry", element.entry_id.map_or_else(|| "Application shell".into(), |id| id.to_string()), colors)}
                            <p class="px-3 py-2 text-[10px] text-muted">"Source identifies the inspection boundary."</p>
                        }
                        if element.metadata_truncated { <p class="px-3 py-2 text-property">"Some metadata exceeded capture limits."</p> }
                    } else { <p class="px-3 py-4 text-muted">"Select an element to inspect its authored styles, layout and source."</p> }
                </section>
            </section>
        }.inspect("inspector.details");

        rake::view! {
            <div id="inspector" class="size-full flex flex-col min-h-0 relative bg-background text-ink text-[12px] leading-[20px]" focus={&self.focus}
                on:key-down={cx.listener(Self::key)}
                on:mouse-move={cx.listener(|this, event: &MouseMoveEvent, window, cx| { if this.dragging { if event.dragging() { this.split = (f32::from(event.position.x) / f32::from(window.viewport_size().width)).clamp(0.2, 0.8); cx.notify(); } else { this.dragging = false; } } })}
                on:mouse-up={cx.listener(|this, _, _, _| this.dragging = false)}>
                <div class="flex flex-row items-center h-[32px] shrink-0 bg-bar border-b-1 border-line">
                    <div class="flex flex-row items-center gap-1 px-2 text-muted">{icon(Icon::Code, colors.muted)}<span>"Rake"</span></div>
                    <span class="w-[1px] h-[16px] bg-line"/>
                    <div class="flex items-center px-3 h-full border-b-2 border-accent text-accent font-semibold">"Elements"</div>
                    <span class="flex-1"/>
                    <button id="targets" aria-label="Select target runtime" class="flex flex-row items-center gap-1 px-2 h-[26px] text-muted hover:bg-field" style={control}
                        on:click={cx.listener(|this, _, _, cx| { this.show_targets = !this.show_targets; cx.notify(); })}><span>{target_name}</span>{icon(Icon::Down, colors.muted)}</button>
                    <span class="w-[1px] h-[16px] bg-line"/>
                    <button id="find" aria-label="Find in element tree" class="w-[30px] h-full hover:bg-field" style={control} on:click={cx.listener(Self::toggle_find)}>{icon(Icon::Search, colors.muted)}</button>
                    <button id="reveal-selection" disabled={!can_reveal} aria-label="Reveal selected element in tree" aria-description="Expand its current ancestors and scroll it into view. A replaced capture cannot be retargeted."
                        class="w-[30px] h-full hover:bg-field opacity-100" class:opacity-35={!can_reveal} style={control} on:click={cx.listener(|this, _, window, cx| this.reveal_selection(window, cx))}>{icon(Icon::Reveal, colors.muted)}</button>
                    <button id="pick" aria-label="Pick element in application" disabled={!model.instances.iter().any(|i|Some(i.info.runtime_id)==model.runtime && i.info.element_interaction)} class="px-2 h-full hover:bg-field" style={control}
                        on:click={cx.listener(|this, _, _, cx| {let mut model=this.model.borrow_mut();model.picking = !model.picking;model.mode_pending=Some(model.picking);model.hover=None;cx.notify();})}>{if model.picking {"Cancel pick"} else {"Pick"}}</button>
                    <button id="pause" aria-label={if model.paused { "Resume observation" } else { "Pause observation" }} class="w-[30px] h-full hover:bg-field" style={control} on:click={cx.listener(|this, _, _, cx| { let mut model = this.model.borrow_mut(); model.paused = !model.paused; cx.notify(); })}>{icon(if model.paused { Icon::Play } else { Icon::Pause }, if model.paused { colors.accent } else { colors.muted })}</button>
                    <button id="refresh" aria-label="Refresh capture" class="w-[30px] h-full hover:bg-field" style={control} on:click={cx.listener(|this, _, _, cx| { this.model.borrow_mut().refresh = true; cx.notify(); })}>{icon(Icon::Refresh, colors.muted)}</button>
                    <button id="theme" aria-label="Toggle theme" class="w-[30px] h-full hover:bg-field" style={control} on:click={cx.listener(|this, _, _, cx| { this.light = !this.light; rake::theme::install(cx, Palette::new(this.light)); this.filter.update(cx, |input, cx| input.set_light(this.light, cx)); this.style_filter.update(cx, |input, cx| input.set_light(this.light, cx)); cx.notify(); })}>{icon(Icon::Theme, colors.muted)}</button>
                </div>
                if let Some(error) = error { <p class="px-3 py-1 text-property border-b-1 border-line">{error}</p> }
                <div class="flex flex-1 min-h-0 min-w-0" class:flex-col={narrow} class:flex-row={!narrow}>
                    {tree_pane}
                    if !narrow {
                        <div id="divider" tabindex={0} aria-label="Resize inspection panes" class="w-[4px] h-full shrink-0 border-l-1 border-line hover:bg-field" style={|s| s.cursor(CursorStyle::ResizeColumn).track_focus(&self.divider)}
                            on:mouse-down={cx.listener(|this, _, window, cx| { this.dragging = true; window.focus(&this.divider, cx); })}
                            on:key-down={cx.listener(|this, event: &KeyDownEvent, _, cx| { match event.keystroke.key.as_str() { "left" => this.split = (this.split - 0.025).max(0.2), "right" => this.split = (this.split + 0.025).min(0.8), "home" => this.split = 0.2, "end" => this.split = 0.8, _ => return } cx.stop_propagation(); cx.notify(); })}/>
                    }
                    {details}
                </div>
                if self.show_coverage {
                    <section class="px-3 py-2 shrink-0 border-t-1 border-line bg-bar text-[11px] text-muted"  aria-label="Inspection coverage">
                        <p>{model.tree.as_ref().map_or_else(|| "Waiting for capture coverage.".into(), |tree| tree.coverage.clone())}</p>
                        <p>"Tree id attributes show semantic IDs. Source links identify inspection boundaries."</p>
                    </section>
                }
                <div class="flex flex-row items-center gap-2 px-2 h-[23px] shrink-0 bg-bar border-t-1 border-line text-[10px] text-muted">
                    <button id="coverage" aria-label="Show capture and coverage details" class="flex flex-row items-center gap-1 h-full hover:text-ink" style={control} on:click={cx.listener(|this, _, _, cx| { this.show_coverage = !this.show_coverage; cx.notify(); })}>{icon(if self.show_coverage { Icon::Down } else { Icon::Right }, colors.muted)}<span>{capture_status}</span></button>
                    <span class="flex-1"/>
                    <span class={if model.disconnected { "w-[6px] h-[6px] rounded-full bg-muted" } else if self.config_error.is_some() || model.error.is_some() { "w-[6px] h-[6px] rounded-full bg-property" } else { "w-[6px] h-[6px] rounded-full bg-status-success" }}/>
                    <span>{status}</span>
                    <button id="disconnect" class="h-full px-[5px] hover:text-ink" style={control} on:click={cx.listener(|this, _, _, cx| { let mut model = this.model.borrow_mut(); model.disconnected = !model.disconnected; let runtime = model.runtime; model.select_runtime(runtime); cx.notify(); })}>{if model.disconnected { "Reconnect" } else { "Disconnect" }}</button>
                </div>
                if self.show_targets {
                    <section id="target-menu" class="flex flex-col p-1 border absolute top-[32px] right-[6px] w-[490px] max-w-[100%] max-h-[220px] overflow-y-scroll bg-background border-line shadow-lg">
                        for instance in &model.instances {
                            <button id={format!("runtime-{}", instance.info.runtime_id)} class="px-3 py-2 justify-start text-[11px] hover:bg-hover" style={control} on:click={ { let runtime = instance.info.runtime_id; cx.listener(move |this, _, _, cx| this.select_runtime(runtime, cx)) } }>{format!("{} · {}\n{}", instance.application_id, instance.info.platform, instance.info.runtime_id)}</button>
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
