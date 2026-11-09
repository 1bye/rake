//! Fieldnotes' application-owned shell, using the shared notebook service and Rake lifecycle.

use crate::model::{MAX_BODY_BYTES, MAX_TITLE_BYTES};
use crate::{CreateNote, EditNote, Library, Note, Notebook, Repository, SaveNotes};

use rake::gpui::{
    self, AnyView, Bounds, ClipboardItem, Context, Entity, Focusable, Task, Window, WindowBounds,
    WindowOptions, div, point, prelude::*, px, rgb, size,
};

use rake::prelude::*;
use rake::router::{GuardReason, MountError};
use rake::ui::input::{Input, InputAccessibility, InputEvent, InputState, Textarea, TextareaState};
use std::rc::Rc;

mod text_keys;
#[cfg(target_family = "wasm")]
mod web_accessibility;

const INK: u32 = 0x252d27;
const MUTED: u32 = 0x858b80;
const GREEN: u32 = 0x49674e;
const LINE: u32 = 0xe3e5dc;
const SEARCH_LABEL: &str = "Search notes";
const TITLE_LABEL: &str = "Note title";
const BODY_LABEL: &str = "Document body";

#[derive(Clone, Route)]
pub(crate) enum Destination {
    #[route(id = "notebook", path = "/")]
    Notebook,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Collection {
    All,
    Pinned,
    Archive,
}

#[rake::data]
struct ReloadNotebook {
    library: Option<Library>,
    revision: u64,
}

impl ReloadState for ReloadNotebook {
    const SCHEMA: &'static str = "fieldnotes.notebook";
    const VERSION: u32 = 1;
}

struct Views;

struct ModelChanged;

#[cfg(target_family = "wasm")]
struct WebFonts;

#[cfg(target_family = "wasm")]
impl gpui::Global for WebFonts {}

impl RouteViews<Destination> for Views {
    fn mount(
        &self,
        _: &Destination,
        page: PageContext<Destination>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<AnyView, MountError> {
        #[cfg(target_family = "wasm")]
        if !cx.has_global::<WebFonts>() {
            cx.text_system()
                .add_fonts(vec![
                    std::borrow::Cow::Borrowed(
                        include_bytes!("../assets/fonts/NotoSansJP-Regular.otf").as_slice(),
                    ),
                    std::borrow::Cow::Borrowed(
                        include_bytes!("../assets/fonts/NotoColorEmoji.ttf").as_slice(),
                    ),
                ])
                .map_err(|error| MountError(format!("Cannot load document fonts: {error}")))?;
            cx.set_global(WebFonts);
        }

        window.set_window_title("Fieldnotes");
        let notebook = page
            .services()
            .get::<Notebook>()
            .map_err(|error| MountError(error.to_string()))?;

        let store = notebook.clone();
        let captured = notebook.clone();

        let restored = page
            .entry()
            .reload_state::<ReloadNotebook>()
            .map_err(|error| MountError(error.to_string()))?;

        if let Some(restored) = restored
            && let Some(library) = restored.library
        {
            notebook
                .restore_pending(library, restored.revision)
                .map_err(|error| MountError(error.to_string()))?;
        }

        page.entry()
            .on_reload(move || {
                ReloadNotebook {
                    library: if captured.is_dirty() {
                        Some(
                            captured
                                .library()
                                .ok_or_else(|| ReloadError::State("Library is opening.".into()))?,
                        )
                    } else {
                        None
                    },
                    revision: captured.revision(),
                }
                .checkpoint()
            })
            .map_err(|error| MountError(error.to_string()))?;
        let guard = notebook.clone();

        page.entry()
            .on_dispose(move |_context| {
                let guard = guard.clone();

                async move {
                    if !guard.is_dirty() {
                        return GuardResult::Allow;
                    }

                    match guard.flush().await {
                        Ok(_) => GuardResult::Allow,
                        Err(error) => {
                            let retry = guard.clone();

                            GuardResult::Ask(DecisionRequest {
                                message: format!("Your latest changes could not be saved: {error}"),
                                choices: vec![
                                    DecisionChoice::proceed("save", "Retry save").prepare(
                                        move || {
                                            let retry = retry.clone();
                                            async move {
                                                retry
                                                    .flush()
                                                    .await
                                                    .map(|_| ())
                                                    .map_err(|error| GuardReason(error.to_string()))
                                            }
                                        },
                                    ),
                                    DecisionChoice::proceed("discard", "Close without saving"),
                                    DecisionChoice::cancel("cancel", "Keep writing"),
                                ],
                            })
                        }
                    }
                }
            })
            .map_err(|error| MountError(error.to_string()))?;
        text_keys::bind(cx);
        let view = cx.new(|cx| NotesView::new(page.clone(), notebook.clone(), window, cx));

        focus_editor(&view.read(cx).body.clone(), window, cx);
        let weak = view.downgrade();
        let context = window.to_async(cx);

        page.on_event(move |_: ModelChanged| {
            let mut context = context.clone();

            context
                .update(|window, cx| {
                    weak.update(cx, |view, cx| {
                        view.sync(window, cx);
                        cx.notify();
                    })
                })
                .map_err(|error| TaskError::Failed(error.to_string()))?
                .map_err(|error| TaskError::Failed(error.to_string()))
        })
        .map_err(|error| MountError(error.to_string()))?;
        page.activity("fieldnotes.observe", move |run| {
            let store = store.clone();

            async move {
                let mut sequence = u64::MAX;

                loop {
                    store.changed(sequence).await;
                    sequence = store.sequence();
                    run.publish(ModelChanged)?;
                }
            }
        })
        .map_err(|error| MountError(error.to_string()))?;
        let auto = notebook.clone();

        page.activity("fieldnotes.autosave", move |_| {
            let auto = auto.clone();

            async move {
                auto.autosave().await;

                Ok(())
            }
        })
        .map_err(|error| MountError(error.to_string()))?;
        let loaded = notebook.clone();

        view.update(cx, |view, cx| {
            view._load = cx.spawn(async move |view, cx| {
                let result = loaded.load().await;

                if let Err(error) = result {
                    rake::error!(%error, "Notebook could not open");
                }

                let _ = view.update(cx, |_, cx| cx.notify());
            });
        });

        Ok(view.into())
    }
}

struct NotesView {
    page: PageContext<Destination>,
    notebook: Rc<Notebook>,
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    search: Entity<InputState>,
    query: String,
    collection: Collection,
    selected: Option<Note>,
    sequence: u64,
    #[cfg(target_family = "wasm")]
    web_accessibility: Option<web_accessibility::WebAccessibility>,
    _dirty: Rc<dyn Fn(bool)>,
    _load: Task<()>,
    title_limit_reached: bool,
    body_limit_reached: bool,
    copied: bool,
    help: bool,
}

impl NotesView {
    fn new(
        page: PageContext<Destination>,
        notebook: Rc<Notebook>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = text_input(TITLE_LABEL, "Untitled note", window, cx);
        let view = cx.weak_entity();

        title.update(cx, |title, cx| {
            let view = view.clone();

            title.set_validator(
                move |value, cx| {
                    if value.len() <= MAX_TITLE_BYTES {
                        return true;
                    }

                    let view = view.clone();

                    cx.defer(move |cx| {
                        let _ = view.update(cx, |view, cx| {
                            view.title_limit_reached = true;
                            cx.notify();
                        });
                    });

                    false
                },
                cx,
            );
        });

        cx.subscribe_in(&title, window, |this, input, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.title_limit_reached = false;
                let value = input.read(cx).value();

                if let Err(error) =
                    update_selected(&this.notebook, this.selected.as_mut(), &value, true)
                {
                    rake::warn!(%error, "Could not edit note title");
                    this.sync(window, cx);
                    let value = this.selected.as_ref().map_or("", |note| &note.title);

                    input.update(cx, |input, cx| input.set_value(value, window, cx));
                }

                cx.notify();
            }
        })
        .detach();
        let body = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .grapheme_editing()
                .max_bytes(MAX_BODY_BYTES, move |cx| {
                    let view = view.clone();

                    cx.defer(move |cx| {
                        let _ = view.update(cx, |view, cx| {
                            view.body_limit_reached = true;
                            cx.notify();
                        });
                    });
                })
                .placeholder("Start with a thought…");

            state.set_accessibility(
                InputAccessibility::new(Some(gpui::accesskit::Role::MultilineTextInput))
                    .with_label(Some(BODY_LABEL.into()))
                    .with_placeholder(Some("Start with a thought…".into())),
            );

            state
        });

        cx.subscribe_in(&body, window, |this, input, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.body_limit_reached = false;
                let value = input.read(cx).value();

                if let Err(error) =
                    update_selected(&this.notebook, this.selected.as_mut(), &value, false)
                {
                    rake::warn!(%error, "Could not edit note body");
                    this.sync(window, cx);
                    let value = this.selected.as_ref().map_or("", |note| &note.body);

                    input.update(cx, |input, cx| input.set_value(value, window, cx));
                }

                cx.notify();
            }
        })
        .detach();

        let search = text_input(SEARCH_LABEL, "Search your notes…", window, cx);

        cx.subscribe(&search, |this, input, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = input.read(cx).value().to_string();
                cx.notify();
            }
        })
        .detach();
        #[cfg(target_family = "wasm")]
        let web_accessibility = web_accessibility::WebAccessibility::new(
            [
                search.read(cx).focus_handle(cx),
                title.read(cx).focus_handle(cx),
                body.read(cx).focus_handle(cx),
            ],
            window,
            cx,
        );

        let entry = page.entry();

        let dirty = notebook.observe_dirty(move |dirty| {
            let _ = entry.set_dirty(dirty);
        });

        Self {
            page,
            notebook,
            title,
            body,
            search,
            query: String::new(),
            collection: Collection::All,
            selected: None,
            sequence: 0,
            #[cfg(target_family = "wasm")]
            web_accessibility,
            _dirty: dirty,
            _load: Task::ready(()),
            title_limit_reached: false,
            body_limit_reached: false,
            copied: false,
            help: false,
        }
    }

    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.notebook.library().and_then(|library| {
            library
                .selected
                .and_then(|id| library.notes.into_iter().find(|note| note.id == id))
        });

        if let Some(note) = selected {
            // Reset both histories on a document switch, including equal field values.
            // On the same note, only replace fields changed outside the editor.
            let previous = self
                .selected
                .as_ref()
                .filter(|previous| previous.id == note.id);

            let reset = previous.is_none();

            if previous.is_none_or(|previous| previous.title != note.title) {
                self.title.update(cx, |input, cx| {
                    input.set_value(note.title.as_str(), window, cx)
                });
            }

            if previous.is_none_or(|previous| previous.body != note.body) {
                self.body.update(cx, |input, cx| {
                    input.set_value(note.body.as_str(), window, cx)
                });
            }

            if reset {
                self.title_limit_reached = false;
                self.body_limit_reached = false;
                self.copied = false;
            }

            window.set_window_title(&format!("{} — Fieldnotes", note.display_title()));
            self.selected = Some(note);
        } else if self.selected.take().is_some() {
            self.title_limit_reached = false;
            self.body_limit_reached = false;
            self.title
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.body
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    fn dispatch<C: rake::control::ApplicationCommand>(&mut self, command: C) {
        self.sequence += 1;
        let key = format!(
            "fieldnotes-{}-{}",
            self.page.entry().id().map(|id| id.value()).unwrap_or(0),
            self.sequence
        );

        if let Err(error) = self.page.dispatch(command, key) {
            rake::error!(%error, "Notebook command rejected");
        }
    }

    fn new_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.collection = Collection::All;
        self.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.dispatch(CreateNote {
            title: String::new(),
            body: String::new(),
        });
        focus_editor(&self.title, window, cx);
        cx.notify();
    }

    fn key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        let command = modifiers.platform || modifiers.control;

        if event.keystroke.key == "tab" && !command && !modifiers.alt {
            if modifiers.shift {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }

            cx.stop_propagation();

            return;
        }

        if command {
            match event.keystroke.key.as_str() {
                "n" => self.new_note(window, cx),
                "s" => self.dispatch(SaveNotes {
                    checkpoint_only: false,
                }),
                "f" | "k" => focus_editor(&self.search, window, cx),
                _ => return,
            }

            cx.stop_propagation();
        }
    }

    fn new_note_click(
        &mut self,
        _: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_note(window, cx);
    }
}

fn fill_input(input: impl IntoElement) -> impl IntoElement {
    div().size_full().min_h_0().flex().child(input)
}

fn input_limit_notice(
    id: &'static str,
    semantic_id: &'static str,
    message: &'static str,
) -> impl IntoElement {
    div()
        .id(id)
        .role(gpui::accesskit::Role::Alert)
        .aria_label(message)
        .px_8()
        .py_3()
        .bg(rgb(0xffefe4))
        .text_color(rgb(0x965639))
        .child(message)
        .inspect(semantic_id)
}

fn text_input(
    label: &'static str,
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut gpui::App,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx)
            .grapheme_editing()
            .line_breaks_as_spaces()
            .placeholder(placeholder);

        state.set_accessibility(
            InputAccessibility::new(Some(gpui::accesskit::Role::TextInput))
                .with_label(Some(label.into()))
                .with_placeholder(Some(placeholder.into())),
        );

        state
    })
}

fn focus_editor<T: Focusable>(editor: &Entity<T>, window: &mut Window, cx: &mut gpui::App) {
    let focus = editor.read(cx).focus_handle(cx);

    window.focus(&focus, cx);
}

fn update_selected(
    store: &Notebook,
    loaded: Option<&mut Note>,
    text: &str,
    title: bool,
) -> Result<(), String> {
    let loaded = loaded.ok_or("Select a note first.")?;
    let library = store.library().ok_or("Library is not ready.")?;

    let note = library
        .notes
        .iter()
        .find(|note| library.selected == Some(loaded.id) && note.id == loaded.id)
        .ok_or("The selected note changed. Please try your edit again.")?;

    let (saved, loaded) = if title {
        (&note.title, &mut loaded.title)
    } else {
        (&note.body, &mut loaded.body)
    };

    if saved != loaded {
        return Err("This note changed elsewhere. Its latest version is being loaded; please try your edit again.".into());
    }

    if saved != text {
        store
            .edit(EditNote {
                id: note.id,
                expected_version: note.version,
                title: title.then(|| text.into()),
                body: (!title).then(|| text.into()),
            })
            .map_err(|error| error.to_string())?;
        *loaded = text.into();
    }

    Ok(())
}

fn relative_date(ms: u64) -> String {
    let age = crate::model::now_ms().saturating_sub(ms) / 1000;

    match age {
        0..=59 => "Just now".into(),
        60..=3599 => format!("{} min ago", age / 60),
        3600..=86399 => "Today".into(),
        86400..=172799 => "Yesterday".into(),
        _ => format!("{} days ago", age / 86400),
    }
}

impl Render for NotesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(window, cx);
        let library = self.notebook.library();
        let search_focus = self.search.read(cx).focus_handle(cx);
        let title_focus = self.title.read(cx).focus_handle(cx);
        let body_focus = self.body.read(cx).focus_handle(cx);

        #[cfg(target_family = "wasm")]
        if let Some(accessibility) = &self.web_accessibility {
            accessibility.sync(window, library.is_some(), self.selected.is_some());
        }

        let error = self.notebook.error();
        let notice = self.notebook.notice();
        let dirty = self.notebook.is_dirty();
        let query = self.query.to_lowercase();

        let selected = library.as_ref().and_then(|library| {
            library
                .notes
                .iter()
                .find(|note| Some(note.id) == library.selected)
        });

        let mut notes: Vec<_> = library
            .as_ref()
            .into_iter()
            .flat_map(|library| &library.notes)
            .filter(|note| match self.collection {
                Collection::All => !note.archived,
                Collection::Pinned => note.pinned && !note.archived,
                Collection::Archive => note.archived,
            })
            .filter(|note| {
                query.is_empty()
                    || note.title.to_lowercase().contains(&query)
                    || note.body.to_lowercase().contains(&query)
            })
            .collect();

        notes.sort_by_key(|note| {
            (
                std::cmp::Reverse(note.modified_ms),
                std::cmp::Reverse(note.id),
            )
        });
        let collections = [
            (
                Collection::All,
                "All notes",
                library.as_ref().map_or(0, |library| {
                    library.notes.iter().filter(|note| !note.archived).count()
                }),
                "notes.all",
            ),
            (
                Collection::Pinned,
                "Pinned",
                library.as_ref().map_or(0, |library| {
                    library
                        .notes
                        .iter()
                        .filter(|note| note.pinned && !note.archived)
                        .count()
                }),
                "notes.pinned",
            ),
            (
                Collection::Archive,
                "Archive",
                library.as_ref().map_or(0, |library| {
                    library.notes.iter().filter(|note| note.archived).count()
                }),
                "notes.archive",
            ),
        ];

        let font = gpui::Font {
            fallbacks: cfg!(target_family = "wasm").then(|| {
                // shortcut: Cosmic selects emoji automatically until GPUI's explicit
                // family loader stops rejecting fonts without a Latin 'm' glyph.
                gpui::FontFallbacks::from_fonts(vec!["Noto Sans JP".into()])
            }),
            ..gpui::font("IBM Plex Sans")
        };

        rake::view! {
            <div
                id={
                    if library.is_some() {
                        "notes.workspace"
                    } else {
                        "notes.startup"
                    }
                }
                class="size-full min-h-0 flex flex-row text-[13px]"
                style={
                    |s|
                        s.font(font)
                            .text_color(rgb(INK))
                            .bg(rgb(0xfffefa))
                            .when(library.is_none(), |s|
                                s.flex_col().justify_center().items_center().gap_4().p_12())
                    }
                key-context="Fieldnotes"
                on:key-down={cx.listener(Self::key)}
            >
                if library.is_some() {
                    <aside
                        id="notes.sidebar"
                        aria-label="Workspace"
                        class="w-[190px] h-full shrink-0 flex flex-col border-r-1 p-5"
                        style={|s| s.bg(rgb(0xf0f1e9)).border_color(rgb(LINE))}
                    >
                        <header class="flex flex-row items-center gap-3 mb-8">
                            <div
                                class="size-9 rounded-lg flex flex-row items-center justify-center text-[24px]"
                                style={|s| s.bg(rgb(GREEN)).text_color(rgb(0xffffff))}
                            >
                                "f."
                            </div>
                            <h1 class="text-[20px] font-semibold">"Fieldnotes"</h1>
                        </header>
                        <div
                            class="text-[10px] font-semibold mb-3"
                            style={|s| s.text_color(rgb(MUTED))}
                        >
                            "YOUR WORKSPACE"
                        </div>
                        <nav
                            id="notes.collections"
                            aria-label="Note collections"
                            class="flex flex-col"
                        >
                            for (collection, label, count, id) in collections {
                                <button
                                    id={id}
                                    aria-label={label}
                                    class="w-full flex flex-row items-center justify-between px-3 py-3 mb-1 rounded-md"
                                    style={
                                        |s|
                                            s.bg(rgb(if collection == self.collection {
                                                0xe0e6da
                                            } else {
                                                0xf0f1e9
                                            }))
                                            .text_color(rgb(
                                                if collection == self.collection {
                                                    GREEN
                                                } else {
                                                    0x70796b
                                                }
                                            ))
                                        }
                                    on:click={
                                        cx.listener(move |this, _, _, cx| {
                                            this.collection = collection;
                                            cx.notify();
                                        })
                                    }
                                >
                                    {label}
                                    <span class="text-[11px]">{count.to_string()}</span>
                                </button>
                            }
                        </nav>
                        <button
                            id="notes.new"
                            aria-label="New note"
                            class="mt-5 px-3 py-3 rounded-md"
                            style={|s| s.bg(rgb(GREEN)).text_color(rgb(0xffffff))}
                            on:click={cx.listener(Self::new_note_click)}
                        >
                            "＋  New note"
                        </button>
                        <div class="flex-1" />
                        <p class="text-[11px] leading-[22px]" style={|s| s.text_color(rgb(MUTED))}>
                            {
                                if cfg!(target_os = "macos") {
                                    "⌘ N   New note\n⌘ F   Find a note\n⌘ S   Save now"
                                } else {
                                    "Ctrl N   New note\nCtrl F   Find a note\nCtrl S   Save now"
                                }
                            }
                        </p>
                        <footer
                            class="mt-5 pt-4 border-t-1 text-[11px]"
                            style={|s| s.border_color(rgb(LINE)).text_color(rgb(MUTED))}
                        >
                            "●  On this device"
                            <button
                                id="notes.help"
                                class="mt-2 text-[11px]"
                                aria-expanded={self.help}
                                style={|s| s.text_color(rgb(GREEN))}
                                on:click={
                                    cx.listener(|this, _, _, cx| {
                                        this.help = !this.help;
                                        cx.notify();
                                    })
                                }
                            >
                                "About & keyboard shortcuts"
                            </button>
                        </footer>
                    </aside>
                    <section
                        id="notes.list"
                        aria-label="Notes"
                        class="w-[286px] h-full shrink-0 flex flex-col border-r-1"
                        style={|s| s.bg(rgb(0xfafaf6)).border_color(rgb(LINE))}
                    >
                        <header class="p-5 pb-4">
                            <div class="flex flex-row items-center justify-between mb-4">
                                <h2 class="text-[22px] font-semibold">
                                    {
                                        match self.collection {
                                            Collection::All => "All notes",
                                            Collection::Pinned => "Pinned",
                                            Collection::Archive => "Archive"
                                        }
                                    }
                                </h2>
                                <span class="text-[11px]" style={|s| s.text_color(rgb(MUTED))}>
                                    {notes.len().to_string()}
                                </span>
                            </div>
                            <div
                                id="notes.search"
                                focus={&search_focus}
                                class="h-9 px-3 py-1 rounded-md flex text-[13px] leading-[22px]"
                                style={|s| s.bg(rgb(0xeeefe8))}
                            >
                                {fill_input(Input::new(&self.search))}
                            </div>
                        </header>
                        <div id="notes.entries" class="flex-1 min-h-0 overflow-y-scroll px-3 pb-3">
                            if notes.is_empty() {
                                <p
                                    class="p-4 mt-8 leading-[24px]"
                                    style={|s| s.text_color(rgb(MUTED))}
                                >
                                    {
                                        if query.is_empty() {
                                            "A little room for something new.\nCreate a note to get started."
                                        } else {
                                            "No notes found.\nTry a different word."
                                        }
                                    }
                                </p>
                            }
                            for note in notes {
                                <button
                                    id={format!("notes.note.{}", note.id)}
                                    aria-label={note.display_title().to_string()}
                                    class="w-full shrink-0 flex flex-col items-start px-4 py-4 mb-2 rounded-lg border-1"
                                    style={
                                        |s|
                                            s.border_color(rgb(
                                                if library.as_ref().is_some_and(|library|
                                                    library.selected == Some(note.id)) {
                                                    0xcdd6c3
                                                } else {
                                                    0xfafaf6
                                                }
                                            ))
                                            .bg(rgb(
                                                if library.as_ref().is_some_and(|library|
                                                    library.selected == Some(note.id)) {
                                                    0xffffff
                                                } else {
                                                    0xfafaf6
                                                }
                                            ))
                                        }
                                    on:click={
                                        {
                                            let id = note.id;
                                            cx.listener(move |this, _, window, cx| {
                                                let _ = this.notebook.select(id);
                                                this.sync(window, cx);
                                                cx.notify();
                                            })
                                        }
                                    }
                                >
                                    <div
                                        class="w-full flex flex-row items-start justify-between gap-2 mb-2"
                                    >
                                        <div class="font-semibold text-[14px]">
                                            {note.display_title().to_string()}
                                        </div>
                                        if note.pinned {
                                            <span
                                                class="text-[11px]"
                                                style={|s| s.text_color(rgb(GREEN))}
                                            >
                                                "◆"
                                            </span>
                                        }
                                    </div>
                                    <p
                                        class="text-[12px] leading-[19px]"
                                        style={|s| s.text_color(rgb(0x92968c))}
                                    >
                                        {
                                            {
                                                let preview = note
                                                    .body
                                                    .split_whitespace()
                                                    .take(18)
                                                    .collect::<Vec<_>>()
                                                    .join(" ");
                                                if preview.is_empty() {
                                                    "Your next thought starts here…".into()
                                                } else {
                                                    preview
                                                }
                                            }
                                        }
                                    </p>
                                    <span
                                        class="mt-3 text-[10px]"
                                        style={|s| s.text_color(rgb(MUTED))}
                                    >
                                        {
                                            format!(
                                                "{}  ·  {} words",
                                                relative_date(note.modified_ms),
                                                note.word_count()
                                            )
                                        }
                                    </span>
                                </button>
                            }
                        </div>
                    </section>
                    <main id="notes.document" class="flex-1 min-w-0 h-full flex flex-col">
                        <header
                            id="notes.document-header"
                            class="h-[68px] shrink-0 px-8 flex flex-row items-center justify-between border-b-1"
                            style={|s| s.border_color(rgb(LINE))}
                        >
                            <span class="text-[11px]" style={|s| s.text_color(rgb(MUTED))}>
                                "PERSONAL  /  FIELDNOTES"
                            </span>
                            <div class="flex flex-row items-center gap-4">
                                <span
                                    class="text-[11px]"
                                    style={
                                        |s|
                                            s.text_color(rgb(if error.is_some() {
                                                0xb45d43
                                            } else {
                                                GREEN
                                            }))
                                        }
                                >
                                    {
                                        if error.is_some() {
                                            "●  Save needs attention"
                                        } else if dirty {
                                            "○  Saving…"
                                        } else {
                                            "●  Saved on this device"
                                        }
                                    }
                                </span>
                                <button
                                    id="notes.save"
                                    class="px-3 py-2 rounded-md border-1"
                                    style={|s| s.border_color(rgb(LINE))}
                                    on:click={
                                        cx.listener(|this, _, _, _|
                                            this.dispatch(SaveNotes {
                                                checkpoint_only: false
                                            }))
                                    }
                                >
                                    "Save now"
                                </button>
                            </div>
                        </header>
                        if let Some(error) = &error {
                            <div
                                id="notes.save-error"
                                role={gpui::accesskit::Role::Alert}
                                class="px-8 py-3"
                                style={|s| s.bg(rgb(0xffefe4)).text_color(rgb(0x965639))}
                            >
                                {error.clone()}
                            </div>
                        }
                        if let Some(notice) = &notice {
                            <div
                                id="notes.recovery"
                                class="px-8 py-3 flex flex-row justify-between gap-3"
                                style={|s| s.bg(rgb(0xeff3e9)).text_color(rgb(GREEN))}
                            >
                                {notice.clone()}
                                <button
                                    id="notes.dismiss-recovery"
                                    on:click={
                                        cx.listener(|this, _, _, _| this.notebook.dismiss_notice())
                                    }
                                >
                                    "Dismiss"
                                </button>
                            </div>
                        }
                        if self.title_limit_reached {
                            {
                                input_limit_notice(
                                    "title-limit",
                                    "notes.title-limit",
                                    "The title is limited to 1,024 bytes."
                                )
                            }
                        }
                        if self.body_limit_reached {
                            {
                                input_limit_notice(
                                    "body-limit",
                                    "notes.body-limit",
                                    "This note has reached its editing limit. Copy or export it before adding more."
                                )
                            }
                        }
                        if self.help {
                            <section
                                id="notes.about"
                                class="mx-8 mt-5 p-5 rounded-lg leading-[24px]"
                                style={|s| s.bg(rgb(0xf0f1e9))}
                            >
                                <p>
                                    "A quiet place to think. Fieldnotes saves notes on this device, without an account. Edits get a recovery checkpoint, then an automatic save after a short pause. Use Save now before unplugging a drive or copying the library. Archive keeps notes until you restore them."
                                </p>
                                <p class="mt-2" style={|s| s.text_color(rgb(GREEN))}>
                                    "Keyboard: Cmd/Ctrl N · new note   /   F · search   /   S · save   /   Z · undo"
                                </p>
                            </section>
                        }
                        if let Some(note) = selected {
                            <article
                                id="notes.document-content"
                                class="flex-1 min-h-0 flex flex-col px-[54px] pt-10 pb-5"
                            >
                                <header class="flex flex-row items-center justify-between mb-5">
                                    <span class="text-[11px]" style={|s| s.text_color(rgb(MUTED))}>
                                        {
                                            format!(
                                                "{}  ·  PERSONAL NOTE",
                                                relative_date(note.modified_ms)
                                            )
                                        }
                                    </span>
                                    <div
                                        class="flex flex-row gap-4 text-[11px]"
                                        style={|s| s.text_color(rgb(GREEN))}
                                    >
                                        <button
                                            id="notes.pin"
                                            aria-label={
                                                if note.pinned {
                                                    "Unpin note"
                                                } else {
                                                    "Pin note"
                                                }
                                            }
                                            on:click={
                                                {
                                                    let id = note.id;
                                                    cx.listener(move |this, _, _, _| {
                                                        let _ = this.notebook.pin(id);
                                                    })
                                                }
                                            }
                                        >
                                            {
                                                if note.pinned {
                                                    "◆ Pinned"
                                                } else {
                                                    "◇ Pin note"
                                                }
                                            }
                                        </button>
                                        <button
                                            id="notes.archive-note"
                                            aria-label={
                                                if note.archived {
                                                    "Restore note"
                                                } else {
                                                    "Archive note"
                                                }
                                            }
                                            on:click={
                                                {
                                                    let id = note.id;
                                                    cx.listener(move |this, _, _, _| {
                                                        let _ = this.notebook.archive(id);
                                                    })
                                                }
                                            }
                                        >
                                            {
                                                if note.archived { "Restore" } else { "Archive" }
                                            }
                                        </button>
                                    </div>
                                </header>
                                <div
                                    id="notes.title"
                                    focus={&title_focus}
                                    class="h-[60px] shrink-0"
                                >
                                    {fill_input(Input::new(&self.title))}
                                </div>
                                <div
                                    class="w-12 h-[2px] mt-3 mb-7"
                                    style={|s| s.bg(rgb(0xd6dece))}
                                />
                                <div id="notes.body" focus={&body_focus} class="flex-1 min-h-0">
                                    {fill_input(Textarea::new(&self.body))}
                                </div>
                                <footer
                                    class="flex flex-row justify-between items-center pt-5 border-t-1 text-[11px]"
                                    style={|s| s.border_color(rgb(LINE)).text_color(rgb(MUTED))}
                                >
                                    <span>
                                        {
                                            format!(
                                                "{} words   ·   {} characters",
                                                note.word_count(),
                                                note.body.chars().count()
                                            )
                                        }
                                    </span>
                                    <button
                                        id="notes.copy"
                                        aria-label="Copy as Markdown"
                                        style={|s| s.text_color(rgb(GREEN))}
                                        on:click={
                                            cx.listener(|this, _, _, cx| {
                                                if let Some(library) = this.notebook.library()
                                                    && let Some(note) =
                                                        library.notes.iter().find(|note|
                                                            Some(note.id) == library.selected)
                                                {
                                                    cx.write_to_clipboard(
                                                        ClipboardItem::new_string(format!(
                                                            "# {}\n\n{}\n",
                                                            note.display_title(),
                                                            note.body
                                                        ))
                                                    );
                                                    this.copied = true;
                                                    cx.notify();
                                                }
                                            })
                                        }
                                    >
                                        {
                                            if self.copied {
                                                "Copied ✓"
                                            } else {
                                                "Copy as Markdown ↗"
                                            }
                                        }
                                    </button>
                                </footer>
                            </article>
                        } else {
                            <div
                                class="flex-1 flex flex-row items-center justify-center"
                                style={|s| s.text_color(rgb(MUTED))}
                            >
                                "Choose a note, or start with something new."
                            </div>
                        }
                    </main>
                } else {
                    <h1 class="text-[32px] font-semibold">"Fieldnotes"</h1>
                    <p class="max-w-[500px]" style={|s| s.text_color(rgb(MUTED))}>
                        {
                            error
                                .clone()
                                .unwrap_or_else(|| "Opening your workspace…".into())
                        }
                    </p>
                    if error.is_some() {
                        <button
                            id="notes.retry-open"
                            class="px-4 py-2 rounded-md"
                            style={|s| s.bg(rgb(GREEN)).text_color(rgb(0xffffff))}
                            on:click={
                                cx.listener(|this, _, _, cx| {
                                    let notebook = this.notebook.clone();
                                    this._load = cx.spawn(async move |view, cx| {
                                        let _ = notebook.load().await;
                                        let _ = view.update(cx, |_, cx| cx.notify());
                                    });
                                })
                            }
                        >
                            "Try again"
                        </button>
                    }
                    <p class="text-[12px]" style={|s| s.text_color(rgb(MUTED))}>
                        "Your existing files will never be replaced by an empty library."
                    </p>
                }
            </div>
        }
    }
}

pub(crate) fn application() -> rake::App<Destination> {
    let repository = Repository::application();

    let delay = std::env::var("RAKE_NOTES_AUTOSAVE_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (100..=60_000).contains(value));

    let notebook = Notebook::configured(
        repository,
        std::time::Duration::from_millis(delay.unwrap_or(700)),
    );

    let config = RouterConfig {
        default_mode: TabMode::Live,
        ..Default::default()
    };

    rake::App::new("dev.rake.fieldnotes")
        .service(notebook)
        .expect("one notebook service")
        .routes(Views)
        .router(config)
        .workspace_chrome(false)
        .control(ControlConfig::FromEnvironment)
        .commands(Notebook::commands().expect("unique notebook commands"))
        .window_options(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                point(px(80.), px(80.)),
                size(px(1220.), px(820.)),
            ))),
            window_min_size: Some(size(px(960.), px(640.))),
            ..Default::default()
        })
}
