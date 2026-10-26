//! Experimental HTML-like views compiled directly into GPUI elements.

use rake::App;
use rake::gpui::prelude::*;
use rake::gpui::{Context, Entity, FocusHandle, KeyDownEvent, ScrollHandle, Window, div, rgb};

struct HistoryItem {
    count: usize,
    clicks: usize,
    focus: FocusHandle,
}

impl Render for HistoryItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <li class="px-3 py-1 rounded-md" style={|s| s.bg(rgb(0x18232b))}>
                <button focus={&self.focus}
                    aria-label={format!("Count {}, row clicks {}", self.count, self.clicks)}
                    style={|s| s.focus(|s| s.bg(rgb(0x365849)))}
                    onclick={cx.listener(|this, _, _, cx| {
                        this.clicks += 1;
                        cx.notify();
                    })}>
                    {format!("Reached {} · row clicks: {}", self.count, self.clicks)}
                </button>
            </li>
        }
    }
}

struct Counter {
    count: usize,
    history: Vec<Entity<HistoryItem>>,
    show_history: bool,
    newest_first: bool,
    focus: FocusHandle,
    history_scroll: ScrollHandle,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <div class="size-full flex flex-col items-center justify-center p-6"
                focus={&self.focus}
                style={|s| s.bg(rgb(0x18232b)).text_color(rgb(0xf0f4e8))}
                onkeydown={cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    let mut modifiers = event.keystroke.modifiers;
                    modifiers.shift = false;

                    if event.keystroke.key == "tab" && !modifiers.modified() {
                        if event.keystroke.modifiers.shift {
                            window.focus_prev(cx);
                        } else {
                            window.focus_next(cx);
                        }

                        window.prevent_default();
                        cx.stop_propagation();
                    } else if event.keystroke.key == "r" && !modifiers.modified() {
                        this.newest_first = !this.newest_first;
                        cx.notify();
                    }
                })}>
                <section class="w-full flex flex-col gap-4 p-6 rounded-xl"
                    style={|s| s.max_w(rake::gpui::px(560.0)).bg(rgb(0x22323d))}>
                    <h1>"HTML-like views. Native GPUI."</h1>
                    <p class="text-sm">"Tags, Rust state, and direct GPUI rendering."</p>
                    <p class="text-3xl font-bold">{format!("Count: {}", self.count)}</p>

                    if self.count == 0 {
                        <p>"Click Increment to get started."</p>
                    } else if self.count == 1 {
                        <p>"Your first click is in the history."</p>
                    } else {
                        <p>{format!("You have counted {} times.", self.count)}</p>
                    }

                    <div class="flex flex-row gap-2">
                        <button id="increment" aria-label="Increment counter"
                            class="px-4 py-2 rounded-md font-semibold"
                            style={|s| s.bg(rgb(0x365849)).hover(|s| s.bg(rgb(0x46715e)))
                                .focus(|s| s.bg(rgb(0x46715e)))}
                            onclick={cx.listener(|this, _, _, cx| {
                                this.count += 1;
                                let count = this.count;

                                this.history.push(cx.new(|cx| HistoryItem {
                                    count,
                                    clicks: 0,
                                    focus: cx.focus_handle().tab_stop(true),
                                }));

                                if this.history.len() > 5 {
                                    this.history.remove(0);
                                }

                                cx.notify();
                            })}>
                            "Increment"
                        </button>
                        <button id="reset" class="px-4 py-2 rounded-md border"
                            style={|s| s.border_color(rgb(0x61717c)).focus(|s| s.bg(rgb(0x365849)))}
                            onclick={cx.listener(|this, _, _, cx| {
                                this.count = 0;
                                this.history.clear();
                                cx.notify();
                            })}>
                            "Reset"
                        </button>
                        <button id="toggle-history" class="px-4 py-2 rounded-md border"
                            aria-label={if self.show_history { "Hide history" } else { "Show history" }}
                            style={|s| s.border_color(rgb(0x61717c)).focus(|s| s.bg(rgb(0x365849)))}
                            onclick={cx.listener(|this, _, _, cx| {
                                this.show_history = !this.show_history;
                                cx.notify();
                            })}>
                            {if self.show_history { "Hide history" } else { "Show history" }}
                        </button>
                    </div>

                    if self.show_history {
                        <section class="flex flex-col gap-2">
                            <div class="flex flex-row items-center justify-between">
                                <h2 class="text-lg">"Recent counts"</h2>
                                <button id="reverse-history" class="text-sm px-3 py-1 rounded-md border"
                                    style={|s| s.border_color(rgb(0x61717c))}
                                    onclick={cx.listener(|this, _, _, cx| {
                                        this.newest_first = !this.newest_first;
                                        cx.notify();
                                    })}>
                                    "Reverse order"
                                </button>
                            </div>
                            <p class="text-xs">"Click a row, then press R: its state and focus follow it."</p>
                            if self.history.is_empty() {
                                <p class="text-sm">"No entries yet."</p>
                            }
                            <ul class="flex flex-col gap-1" scroll={&self.history_scroll}
                                style={|s| s.max_h(rake::gpui::px(150.)).overflow_y_scroll()}>
                                if self.newest_first {
                                    for item in self.history.iter().rev() {
                                        <view entity={&item}/>
                                    }
                                } else {
                                    for item in &self.history {
                                        <view entity={&item}/>
                                    }
                                }
                            </ul>
                        </section>
                    }

                    {div().text_xs().child("Existing GPUI elements embed with {expression}.")}
                </section>
            </div>
        }
    }
}

/// Configures the same macro-authored view on native and browser hosts.
pub fn application() -> App {
    App::new("com.example.rake.view").root(|_, window, cx| {
        let focus = cx.focus_handle();

        window.focus(&focus, cx);

        cx.new(|_| Counter {
            count: 0,
            history: Vec::new(),
            show_history: true,
            newest_first: true,
            focus,
            history_scroll: ScrollHandle::new(),
        })
        .into()
    })
}

#[rake::app]
fn app() -> App {
    application()
}
