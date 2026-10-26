//! Experimental HTML-like views compiled directly into GPUI elements.

use rake::App;
use rake::gpui::prelude::*;
use rake::gpui::{Context, FocusHandle, Window, div, rgb};

struct Counter {
    count: usize,
    history: Vec<usize>,
    show_history: bool,
    focus: FocusHandle,
}

impl Render for Counter {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <div class="size-full flex flex-col items-center justify-center p-6"
                style={|s| s.bg(rgb(0x18232b)).text_color(rgb(0xf0f4e8)).track_focus(&self.focus)}
                onkeydown={|event, window, cx| {
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
                    }
                }}>
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
                                this.history.push(this.count);

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
                            <h2 class="text-lg">"Recent counts"</h2>
                            if self.history.is_empty() {
                                <p class="text-sm">"No entries yet."</p>
                            }
                            <ul class="flex flex-col gap-1">
                                for count in self.history.iter().rev() {
                                    <li id={("count", *count)} class="px-3 py-1 rounded-md"
                                        style={|s| s.bg(rgb(0x18232b))}>
                                        {format!("Reached {count}")}
                                    </li>
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
            focus,
        })
        .into()
    })
}

#[rake::app]
fn app() -> App {
    application()
}
