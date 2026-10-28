//! Experimental HTML-like views with retained entities, typed actions and keyboard controls.

use rake::App;
use rake::gpui::prelude::*;
use rake::gpui::{self, Context, Entity, FocusHandle, KeyBinding, ScrollHandle, Window, div, rgb};

use rake::overlay::{
    AnchorState, Dialog, OverlayAnchor, OverlayHost, OverlayHostState, OverlayState, Popover,
};

mod components;
mod documents;

#[derive(Clone, rake::Theme)]
struct Colors {
    surface: gpui::Rgba,
    ink: gpui::Rgba,
    hover: gpui::Rgba,
}

impl Colors {
    fn dark() -> Self {
        Self {
            surface: rgb(0x18232b),
            ink: rgb(0xf0f4e8),
            hover: rgb(0x365849),
        }
    }

    fn light() -> Self {
        Self {
            surface: rgb(0xf0f4e8),
            ink: rgb(0x18232b),
            hover: rgb(0xc7d8ca),
        }
    }
}

rake::theme!(Colors);

gpui::actions!(
    counter,
    [
        /// Increments an unlocked counter.
        Increment,
        /// Resets an unlocked counter.
        Reset,
        /// Toggles the history panel.
        ToggleHistory,
        /// Reverses the retained history rows.
        ReverseHistory,
        /// Toggles counter editing availability.
        ToggleLocked,
        /// Switches the ambient light/dark theme.
        ToggleTheme,
        /// Activates a retained history row.
        BumpRow,
    ]
);

struct HistoryItem {
    count: usize,
    clicks: usize,
    focus: FocusHandle,
}

impl HistoryItem {
    fn bump(&mut self, _: &BumpRow, _: &mut Window, cx: &mut Context<Self>) {
        self.clicks += 1;
        cx.notify();
    }
}

impl Render for HistoryItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <li class="px-3 py-1 rounded-md" style={|s| s.bg(rgb(0x18232b))}>
                <button focus={&self.focus} key-context="HistoryItem"
                    on:BumpRow={cx.listener(Self::bump)}
                    aria-label={format!("Count {}, row clicks {}", self.count, self.clicks)}
                    aria-keyshortcuts="B"
                    style={|s| s.focus_visible(|s| s.bg(rgb(0x365849)))}
                    on:click={|_, window, cx| window.dispatch_action(Box::new(BumpRow), cx)}>
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
    locked: bool,
    dark: bool,
    focus: FocusHandle,
    history_scroll: ScrollHandle,
    overlays: OverlayHostState,
    actions_anchor: AnchorState,
    actions_popup: OverlayState,
    documents_dialog: OverlayState,
    documents: Entity<documents::Documents>,
    actions_hint: Entity<components::ActionsHint>,
}

impl Counter {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        rake::theme::install(cx, Colors::dark());
        let focus = cx.focus_handle();

        window.focus(&focus, cx);

        let overlays = OverlayHostState::new(cx);

        Self {
            actions_anchor: overlays.anchor(),
            actions_popup: overlays.popover(cx),
            documents_dialog: overlays.dialog(cx),
            documents: cx.new(|cx| documents::Documents::new(cx)),
            actions_hint: cx.new(|_| components::ActionsHint),
            overlays,
            count: 0,
            history: Vec::new(),
            show_history: true,
            newest_first: true,
            locked: false,
            dark: true,
            focus,
            history_scroll: ScrollHandle::new(),
        }
    }

    fn increment(&mut self, _: &Increment, _: &mut Window, cx: &mut Context<Self>) {
        if self.locked {
            return;
        }

        self.count += 1;
        let count = self.count;

        self.history.push(cx.new(|cx| HistoryItem {
            count,
            clicks: 0,
            focus: cx.focus_handle(),
        }));

        if self.history.len() > 5 {
            self.history.remove(0);
        }

        cx.notify();
    }

    fn reset(&mut self, _: &Reset, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked || self.count == 0 {
            return;
        }

        self.count = 0;
        // The focused history row may be removed below; keep keyboard dispatch live.
        window.focus(&self.focus, cx);
        self.history.clear();
        cx.notify();
    }

    fn toggle_history(&mut self, _: &ToggleHistory, window: &mut Window, cx: &mut Context<Self>) {
        self.show_history = !self.show_history;

        if !self.show_history {
            window.focus(&self.focus, cx);
        }

        cx.notify();
    }

    fn reverse_history(&mut self, _: &ReverseHistory, _: &mut Window, cx: &mut Context<Self>) {
        if self.history.len() < 2 {
            return;
        }

        self.newest_first = !self.newest_first;
        cx.notify();
    }

    fn toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        self.dark = !self.dark;
        rake::theme::install(
            cx,
            if self.dark {
                Colors::dark()
            } else {
                Colors::light()
            },
        );
        cx.notify();
    }

    fn toggle_locked(&mut self, _: &ToggleLocked, _: &mut Window, cx: &mut Context<Self>) {
        self.locked = !self.locked;
        cx.notify();
    }
}

impl Counter {
    fn content(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let reset_disabled = self.locked || self.count == 0;

        let increment_shortcut = if cfg!(target_os = "macos") {
            "Meta+I"
        } else {
            "Control+I"
        };

        rake::view! {
            <div class="size-full flex flex-col items-center justify-center p-6 bg-surface text-ink"
                role={gpui::accesskit::Role::Group} aria-label="Counter"
                focus={&self.focus} key-context="Counter" tab-navigation={true}
                on:Increment={cx.listener(Self::increment)}
                on:Reset={cx.listener(Self::reset)}
                on:ToggleHistory={cx.listener(Self::toggle_history)}
                on:ReverseHistory={cx.listener(Self::reverse_history)}
                on:ToggleLocked={cx.listener(Self::toggle_locked)}
                on:ToggleTheme={cx.listener(Self::toggle_theme)}>
                <components::Card id={"counter-card".into()}
                    header={
                        <div class="flex flex-col gap-2">
                            <h1>HTML-like views. Native GPUI.</h1>
                            <p class="text-sm">
                                <strong>Typed components.</strong> Retained state. <em>Native actions.</em>
                            </p>
                        </div>
                    }
                    footer={Some(rake::view! {
                        <p class="text-xs">Card header and footer are typed slots; its body contains retained history views.</p>
                    }.into_any_element())}>
                    <components::Badge id={"editing-status".into()}
                        label={if self.locked { "Locked".into() } else { "Editing enabled".into() }}
                        highlighted={self.locked}/>
                    <p class="text-3xl font-bold">Count: {self.count.to_string()}</p>

                    if self.locked {
                        <p>Counter locked. Increment and Reset are disabled.</p>
                    } else if self.count == 0 {
                        <p>Click Increment to get started.</p>
                    } else {
                        <p>You have counted {self.count.to_string()} times.</p>
                    }

                    <p class="text-sm" wrap={true} max-lines={2} ellipsis={true}>
                        <strong>Rich text preview.</strong> Count <span
                            style={gpui::HighlightStyle::color(gpui::rgb_to_hsla(rgb(0xb9dcbd)))}>
                            {self.count.to_string()}
                        </span>: styled runs wrap together in one paragraph, including <em>italic text</em> and Unicode such as café, 你好 and {"🙂"}.
                        This preview stops at two lines, while the complete text remains available to accessibility.
                        More content demonstrates the ellipsis.
                    </p>
                    <p class="text-xs" wrap={false} ellipsis={true}>
                        <span class="font-semibold">Single-line title: </span>
                        A deliberately long document title that demonstrates truncation without splitting the text into separate layout elements or cutting through UTF-8 characters.
                    </p>

                    <div class="flex flex-row flex-wrap gap-2">
                        <button id="increment" disabled={self.locked} aria-label="Increment counter"
                            aria-keyshortcuts={increment_shortcut}
                            class="px-4 py-2 rounded-md font-semibold"
                            style={|s| s.bg(rgb(0x365849)).opacity(if self.locked { 0.4 } else { 1. })
                                .focus_visible(|s| s.bg(rgb(0x46715e)))}
                            on:click={|_, window, cx| window.dispatch_action(Box::new(Increment), cx)}>
                            Increment
                        </button>
                        <button id="reset" disabled={reset_disabled} aria-keyshortcuts="Escape"
                            class="px-4 py-2 rounded-md border"
                            style={|s| s.border_color(rgb(0x61717c)).opacity(if reset_disabled { 0.4 } else { 1. })
                                .focus_visible(|s| s.bg(rgb(0x365849)))}
                            on:click={|_, window, cx| window.dispatch_action(Box::new(Reset), cx)}>
                            Reset
                        </button>
                        <button id="toggle-history" class="px-4 py-2 rounded-md border"
                            aria-label={if self.show_history { "Hide history" } else { "Show history" }}
                            aria-expanded={self.show_history} aria-keyshortcuts="H"
                            style={|s| s.border_color(rgb(0x61717c)).focus_visible(|s| s.bg(rgb(0x365849)))}
                            on:click={|_, window, cx| window.dispatch_action(Box::new(ToggleHistory), cx)}>
                            if self.show_history { Hide history } else { Show history }
                        </button>
                        <OverlayAnchor state={self.actions_anchor.clone()}>
                            <button id="actions" aria-label="More actions" aria-expanded={self.actions_popup.is_open()}
                                aria-description="Open actions or browse a virtualized list of 10,000 documents"
                                class="px-4 py-2 rounded-md border"
                                style={|s| {
                                    let hint = self.actions_hint.clone();

                                    s.border_color(rgb(0x61717c)).focus_visible(|s| s.bg(rgb(0x365849)))
                                        .tooltip(move |_, _| hint.clone().into())
                                }}
                                on:click={cx.listener(|this, _, window, cx| { this.actions_popup.open(window, cx); })}>
                                More actions
                            </button>
                        </OverlayAnchor>
                        <button id="toggle-lock" class="px-4 py-2 rounded-md border"
                            aria-label={if self.locked { "Unlock counter" } else { "Lock counter" }}
                            aria-keyshortcuts="L"
                            style={|s| s.border_color(rgb(0x61717c)).focus_visible(|s| s.bg(rgb(0x365849)))}
                            on:click={|_, window, cx| window.dispatch_action(Box::new(ToggleLocked), cx)}>
                            if self.locked { Unlock counter } else { Lock counter }
                        </button>
                    </div>
                    <button class="px-4 py-2 rounded-md hover:bg-hover focus-visible:bg-hover"
                        aria-keyshortcuts="T"
                        on:click={|_, window, cx| window.dispatch_action(Box::new(ToggleTheme), cx)}>
                        Switch theme
                    </button>
                    <p class="text-xs">"⌘/Ctrl+I: increment · Esc: reset · H: history · R: reorder · L: lock"</p>

                    if self.show_history {
                        <section class="flex flex-col gap-2">
                            <div class="flex flex-row items-center justify-between">
                                <h2 class="text-lg">Recent counts</h2>
                                <button id="reverse-history" disabled={self.history.len() < 2}
                                    aria-keyshortcuts="R" class="text-sm px-3 py-1 rounded-md border"
                                    style={|s| s.border_color(rgb(0x61717c)).opacity(if self.history.len() < 2 { 0.4 } else { 1. })
                                        .focus_visible(|s| s.bg(rgb(0x365849)))}
                                    on:click={|_, window, cx| window.dispatch_action(Box::new(ReverseHistory), cx)}>
                                    Reverse order
                                </button>
                            </div>
                            <p class="text-xs">Click a row, then press R: its state and focus follow it. B activates the focused row.</p>
                            if self.history.is_empty() {
                                <p>No entries yet.</p>
                            }
                            <ul class="flex flex-col gap-1" scroll={&self.history_scroll}
                                style={|s| s.max_h(gpui::px(150.)).overflow_y_scroll()}>
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
                </components::Card>
            </div>
        }
    }
}

impl Render for Counter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.content(cx).into_any_element();

        let footer = rake::view! {
            <div class="flex flex-row gap-2">
                <button class="px-3 py-2 rounded-md border"
                    style={|s| s.border_color(rgb(0x61717c)).focus_visible(|s| s.bg(rgb(0x365849)))}
                    on:click={cx.listener(|this, _, window, cx| {
                        this.documents.update(cx, |documents, cx| documents.jump(4_999, window, cx));
                    })}>Jump to 5,000</button>
                <button class="px-3 py-2 rounded-md border"
                    style={|s| s.border_color(rgb(0x61717c)).focus_visible(|s| s.bg(rgb(0x365849)))}
                    on:click={cx.listener(|this, _, window, cx| { this.documents_dialog.close(window, cx); })}>Close</button>
            </div>
        }.into_any_element();

        rake::view! {
            <OverlayHost state={self.overlays.clone()} body={body}>
                <Popover state={self.actions_popup.clone()} anchor={self.actions_anchor.clone()}>
                    <p class="font-semibold">Document actions</p>
                    <button disabled={self.locked} class="px-3 py-2 rounded-md"
                        style={|s| s.bg(rgb(0x365849)).opacity(if self.locked { 0.4 } else { 1. }).focus_visible(|s| s.bg(rgb(0x526857)))}
                        on:click={cx.listener(|this, _, window, cx| {
                            this.actions_popup.close(window, cx);
                            this.increment(&Increment, window, cx);
                        })}>Increment counter</button>
                    <button class="px-3 py-2 rounded-md border"
                        style={|s| s.border_color(rgb(0x61717c)).focus_visible(|s| s.bg(rgb(0x365849)))}
                        on:click={cx.listener(|this, _, window, cx| { this.documents_dialog.open(window, cx); })}>
                        Browse 10,000 documents
                    </button>
                    <button class="px-3 py-2 rounded-md"
                        style={|s| s.focus_visible(|s| s.bg(rgb(0x365849)))}
                        on:click={cx.listener(|this, _, window, cx| { this.actions_popup.close(window, cx); })}>Close actions</button>
                </Popover>
                <Dialog state={self.documents_dialog.clone()} title={"Document browser".into()} footer={Some(footer)}>
                    <view entity={&self.documents}/>
                </Dialog>
            </OverlayHost>
        }
    }
}

/// Configures the same macro-authored view on native and browser hosts.
pub fn application() -> App {
    App::new("com.example.rake.view").root(|_, window, cx| {
        cx.bind_keys([
            KeyBinding::new("secondary-i", Increment, Some("Counter")),
            KeyBinding::new("escape", Reset, Some("Counter")),
            KeyBinding::new("h", ToggleHistory, Some("Counter")),
            KeyBinding::new("r", ReverseHistory, Some("Counter")),
            KeyBinding::new("l", ToggleLocked, Some("Counter")),
            KeyBinding::new("t", ToggleTheme, Some("Counter")),
            KeyBinding::new("b", BumpRow, Some("HistoryItem")),
        ]);

        cx.new(|cx| Counter::new(window, cx)).into()
    })
}

#[rake::app]
fn app() -> App {
    application()
}

#[cfg(all(test, feature = "view-test-support"))]
mod tests;
