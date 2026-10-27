//! A retained 10,000-row model with a bounded GPUI virtual list and durable selection.

use rake::gpui::{
    self, Context, FocusHandle, SharedString, UniformListScrollHandle, Window, prelude::*,
};

struct Row {
    id: usize,
    title: SharedString,
}

pub(super) struct Documents {
    rows: Vec<Row>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    selected: Option<usize>,
}

impl Documents {
    pub fn new(cx: &Context<Self>) -> Self {
        Self {
            rows: (0..10_000)
                .map(|id| Row {
                    id,
                    title: format!("Document {:05}", id + 1).into(),
                })
                .collect(),
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            selected: None,
        }
    }

    pub fn jump(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let index = index.min(self.rows.len().saturating_sub(1));

        self.selected = self.rows.get(index).map(|row| row.id);
        self.scroll
            .scroll_to_item(index, gpui::ScrollStrategy::Nearest);
        window.focus(&self.focus, cx);
        cx.notify();
    }
}

impl Render for Documents {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <section focus={&self.focus} key-context="Documents" class="flex flex-col gap-2"
                onkeydown={cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                    if event.keystroke.modifiers.modified() { return; }

                    let index = this.selected.unwrap_or(0);
                    let next = match event.keystroke.key.as_str() {
                        "down" => this.selected.map(|index| index.saturating_add(1)).unwrap_or(0),
                        "up" => index.saturating_sub(1),
                        "home" => 0,
                        "end" => this.rows.len().saturating_sub(1),
                        _ => return,
                    };

                    this.jump(next, window, cx);
                    window.prevent_default();
                    cx.stop_propagation();
                })}>
                <p class="text-sm">
                    "10,000 retained records. "
                    {self.selected.map(|id| format!("Selected: {}.", id + 1)).unwrap_or_else(|| "No selection.".into())}
                </p>
                <section style={|s| s.h(gpui::px(280.)).overflow_hidden()}>
                    {gpui::uniform_list("documents", self.rows.len(), cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range.map(|index| {
                            let row = &this.rows[index];
                            let id = row.id;
                            let selected = this.selected == Some(id);

                            rake::view! {
                                <button id={("document", id)} aria-label={row.title.clone()}
                                    style={|s| s.h(gpui::px(32.)).px_3().justify_start()
                                        .when(selected, |s| s.bg(gpui::rgb(0x365849)))
                                        .focus_visible(|s| s.bg(gpui::rgb(0x526857)))}
                                    onclick={cx.listener(move |this, _, window, cx| this.jump(index, window, cx))}>
                                    {row.title.clone()}
                                </button>
                            }
                        }).collect()
                    })).track_scroll(&self.scroll).h(gpui::px(280.)).flex_shrink_0()}
                </section>
                <p class="text-xs">"Scroll or select a row, then use ↑/↓, Home or End. Selection and scroll survive closing this dialog."</p>
            </section>
        }
    }
}
