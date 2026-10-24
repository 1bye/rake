//! Fieldnotes' application-owned shell, using the shared notebook service and Rake lifecycle.

use crate::editor::Editor;
use crate::{CreateNote, EditNote, Library, Notebook, Repository, SaveNotes};

use rake::gpui::{
    self, AnyView, Bounds, ClipboardItem, Context, Entity, Focusable, FontWeight, Task, Window,
    WindowBounds, WindowOptions, div, point, prelude::*, px, rgb, size,
};

use rake::prelude::*;
use rake::router::{GuardReason, MountError};
use std::rc::Rc;

const INK: u32 = 0x252d27;
const MUTED: u32 = 0x858b80;
const GREEN: u32 = 0x49674e;
const LINE: u32 = 0xe3e5dc;

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

impl RouteViews<Destination> for Views {
    fn mount(
        &self,
        _: &Destination,
        page: PageContext<Destination>,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<AnyView, MountError> {
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
    title: Entity<Editor>,
    body: Entity<Editor>,
    search: Entity<Editor>,
    query: Rc<std::cell::RefCell<String>>,
    collection: Collection,
    selected: Option<u64>,
    sequence: u64,
    _dirty: Rc<dyn Fn(bool)>,
    _load: Task<()>,
    copied: bool,
    help: bool,
}

impl NotesView {
    fn new(
        page: PageContext<Destination>,
        notebook: Rc<Notebook>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title_store = notebook.clone();

        let title = cx.new(|cx| {
            Editor::new(
                "Untitled note",
                "Note title",
                32.,
                false,
                move |old, text| update_selected(&title_store, old, text, true),
                cx,
            )
        });

        let body_store = notebook.clone();

        let body = cx.new(|cx| {
            Editor::new(
                "Start with a thought…",
                "Document body",
                16.,
                true,
                move |old, text| update_selected(&body_store, old, text, false),
                cx,
            )
        });

        let query = Rc::new(std::cell::RefCell::new(String::new()));
        let changed_query = query.clone();

        let search = cx.new(|cx| {
            Editor::new(
                "Search your notes…",
                "Search notes",
                13.,
                false,
                move |_, text| {
                    *changed_query.borrow_mut() = text.into();

                    Ok(())
                },
                cx,
            )
        });

        cx.observe(&search, |_, _, cx| cx.notify()).detach();
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
            query,
            collection: Collection::All,
            selected: None,
            sequence: 0,
            _dirty: dirty,
            _load: Task::ready(()),
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
            let reset = self.selected != Some(note.id);

            self.title.update(cx, |editor, cx| {
                editor.set_text(note.title.clone(), reset, cx)
            });
            self.body.update(cx, |editor, cx| {
                editor.set_text(note.body.clone(), reset, cx)
            });

            if reset {
                self.copied = false;
            }

            self.selected = Some(note.id);
            window.set_window_title(&format!("{} — Fieldnotes", note.display_title()));
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
        self.query.borrow_mut().clear();
        self.search
            .update(cx, |editor, cx| editor.set_text(String::new(), true, cx));
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

    fn sidebar(&self, library: &Library, cx: &mut Context<Self>) -> impl IntoElement {
        let all = library.notes.iter().filter(|note| !note.archived).count();

        let pinned = library
            .notes
            .iter()
            .filter(|note| note.pinned && !note.archived)
            .count();

        let archive = library.notes.iter().filter(|note| note.archived).count();

        div()
            .w(px(190.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(0xf0f1e9))
            .border_r_1()
            .border_color(rgb(LINE))
            .p_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .mb_8()
                    .child(
                        div()
                            .w_9()
                            .h_9()
                            .rounded_lg()
                            .bg(rgb(GREEN))
                            .text_color(rgb(0xffffff))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(24.))
                            .child("f."),
                    )
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Fieldnotes"),
                    ),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(MUTED))
                    .mb_3()
                    .child("YOUR WORKSPACE"),
            )
            .children(
                [
                    (Collection::All, "All notes", all, "notes.all"),
                    (Collection::Pinned, "Pinned", pinned, "notes.pinned"),
                    (Collection::Archive, "Archive", archive, "notes.archive"),
                ]
                .into_iter()
                .map(|(collection, label, count, id)| {
                    let selected = collection == self.collection;

                    div()
                        .id(id)
                        .role(gpui::accesskit::Role::Button)
                        .aria_label(label)
                        .tab_index(0)
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_3()
                        .py_3()
                        .mb_1()
                        .rounded_md()
                        .cursor_pointer()
                        .bg(rgb(if selected { 0xe0e6da } else { 0xf0f1e9 }))
                        .text_color(rgb(if selected { GREEN } else { 0x70796b }))
                        .child(label)
                        .child(div().text_size(px(11.)).child(count.to_string()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.collection = collection;
                            cx.notify();
                        }))
                        .inspect(id)
                }),
            )
            .child(
                div()
                    .mt_5()
                    .id("new-note")
                    .role(gpui::accesskit::Role::Button)
                    .aria_label("New note")
                    .tab_index(0)
                    .px_3()
                    .py_3()
                    .rounded_md()
                    .bg(rgb(GREEN))
                    .text_color(rgb(0xffffff))
                    .cursor_pointer()
                    .child("＋  New note")
                    .on_click(cx.listener(Self::new_note_click))
                    .inspect("notes.new"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .line_height(px(22.))
                    .child(if cfg!(target_os = "macos") {
                        "⌘ N   New note\n⌘ F   Find a note\n⌘ S   Save now"
                    } else {
                        "Ctrl N   New note\nCtrl F   Find a note\nCtrl S   Save now"
                    }),
            )
            .child(
                div()
                    .mt_5()
                    .pt_4()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .text_size(px(11.))
                    .text_color(rgb(MUTED))
                    .child("●  On this device"),
            )
            .child(
                div()
                    .id("about-notes")
                    .mt_2()
                    .text_size(px(11.))
                    .text_color(rgb(GREEN))
                    .cursor_pointer()
                    .child("About & keyboard shortcuts")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.help = !this.help;
                        cx.notify();
                    }))
                    .inspect("notes.help"),
            )
    }

    fn new_note_click(
        &mut self,
        _: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_note(window, cx);
    }

    fn list(&self, library: &Library, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.query.borrow().to_lowercase();

        let mut notes: Vec<_> = library
            .notes
            .iter()
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
        let count = notes.len();

        div().w(px(286.)).flex_shrink_0().h_full().flex().flex_col().bg(rgb(0xfafaf6)).border_r_1().border_color(rgb(LINE))
            .child(div().p_5().pb_4().child(div().flex().items_center().justify_between().mb_4()
                .child(div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).child(match self.collection { Collection::All => "All notes", Collection::Pinned => "Pinned", Collection::Archive => "Archive" }))
                .child(div().text_color(rgb(MUTED)).text_size(px(11.)).child(count.to_string())))
                .child(div().h_9().px_3().py_1().rounded_md().bg(rgb(0xeeefe8)).child(self.search.clone()).inspect("notes.search")))
            .child(div().id("note-list").flex_1().min_h_0().overflow_y_scroll().px_3().pb_3()
                .when(notes.is_empty(), |list| list.child(div().p_4().mt_8().text_color(rgb(MUTED)).line_height(px(24.)).child(if query.is_empty() { "A little room for something new.\nCreate a note to get started." } else { "No notes found.\nTry a different word." })))
                .children(notes.into_iter().map(|note| {
                    let id = note.id; let selected = library.selected == Some(id);
                    let preview: String = note.body.split_whitespace().take(18).collect::<Vec<_>>().join(" ");

                    div().id(format!("note-{id}")).role(gpui::accesskit::Role::Button).aria_label(note.display_title().to_string()).tab_index(0).w_full().px_4().py_4().mb_2().rounded_lg().border_1()
                        .border_color(rgb(if selected { 0xcdd6c3 } else { 0xfafaf6 })).bg(rgb(if selected { 0xffffff } else { 0xfafaf6 }))
                        .cursor_pointer().child(div().flex().items_start().justify_between().gap_2().mb_2().child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.)).child(note.display_title().to_string())).when(note.pinned, |row| row.child(div().text_color(rgb(GREEN)).text_size(px(11.)).child("◆"))))
                        .child(div().text_size(px(12.)).text_color(rgb(0x92968c)).line_height(px(19.)).child(if preview.is_empty() { "Your next thought starts here…".into() } else { preview }))
                        .child(div().mt_3().text_size(px(10.)).text_color(rgb(MUTED)).child(format!("{}  ·  {} words", relative_date(note.modified_ms), note.word_count())))
                        .on_click(cx.listener(move |this, _, window, cx| { let _ = this.notebook.select(id);

                            this.sync(window, cx); cx.notify(); }))
                        .inspect(format!("notes.note.{id}"))
                })))
    }
}

fn focus_editor(editor: &Entity<Editor>, window: &mut Window, cx: &mut gpui::App) {
    let focus = editor.read(cx).focus_handle(cx);

    window.focus(&focus, cx);
}

fn update_selected(store: &Notebook, old: &str, text: &str, title: bool) -> Result<(), String> {
    let library = store.library().ok_or("Library is not ready.")?;

    let note = library
        .notes
        .iter()
        .find(|note| Some(note.id) == library.selected)
        .ok_or("Select a note first.")?;

    if (if title { &note.title } else { &note.body }) != old {
        return Err("This note changed elsewhere. Its latest version is being loaded; please try your edit again.".into());
    }

    store
        .edit(EditNote {
            id: note.id,
            expected_version: note.version,
            title: title.then(|| text.into()),
            body: (!title).then(|| text.into()),
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
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

        let mut root = div()
            .size_full()
            .min_h_0()
            .flex()
            .font_family("IBM Plex Sans")
            .text_size(px(13.))
            .text_color(rgb(INK))
            .bg(rgb(0xfffefa))
            .on_key_down(cx.listener(Self::key));

        let Some(library) = library else {
            let error = self.notebook.error();

            return root
                .flex_col()
                .justify_center()
                .items_center()
                .gap_4()
                .p_12()
                .child(
                    div()
                        .text_size(px(32.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Fieldnotes"),
                )
                .child(
                    div().text_color(rgb(MUTED)).max_w(px(500.)).child(
                        error
                            .clone()
                            .unwrap_or_else(|| "Opening your workspace…".into()),
                    ),
                )
                .when(error.is_some(), |root| {
                    root.child(
                        div()
                            .id("retry-open")
                            .px_4()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(GREEN))
                            .text_color(rgb(0xffffff))
                            .cursor_pointer()
                            .child("Try again")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let notebook = this.notebook.clone();

                                this._load = cx.spawn(async move |view, cx| {
                                    let _ = notebook.load().await;
                                    let _ = view.update(cx, |_, cx| cx.notify());
                                });
                            })),
                    )
                })
                .child(
                    div()
                        .text_color(rgb(MUTED))
                        .text_size(px(12.))
                        .child("Your existing files will never be replaced by an empty library."),
                )
                .inspect("notes.startup");
        };

        let selected = library
            .notes
            .iter()
            .find(|note| Some(note.id) == library.selected)
            .cloned();

        let dirty = self.notebook.is_dirty();
        let error = self.notebook.error();
        let notice = self.notebook.notice();

        root = root
            .child(self.sidebar(&library, cx))
            .child(self.list(&library, cx));
        let mut document = div().flex_1().min_w_0().h_full().flex().flex_col();

        document = document.child(
            div()
                .h(px(68.))
                .flex_shrink_0()
                .px_8()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(rgb(LINE))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child("PERSONAL  /  FIELDNOTES"),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_4()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(if error.is_some() { 0xb45d43 } else { GREEN }))
                                .child(if error.is_some() {
                                    "●  Save needs attention"
                                } else if dirty {
                                    "○  Saving…"
                                } else {
                                    "●  Saved on this device"
                                }),
                        )
                        .child(
                            div()
                                .id("save-notes")
                                .role(gpui::accesskit::Role::Button)
                                .aria_label("Save now")
                                .tab_index(0)
                                .px_3()
                                .py_2()
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(LINE))
                                .cursor_pointer()
                                .child("Save now")
                                .on_click(cx.listener(|this, _, _, _| {
                                    this.dispatch(SaveNotes {
                                        checkpoint_only: false,
                                    })
                                }))
                                .inspect("notes.save"),
                        ),
                ),
        );

        if let Some(error) = error {
            document = document.child(
                div()
                    .px_8()
                    .py_3()
                    .bg(rgb(0xffefe4))
                    .text_color(rgb(0x965639))
                    .child(error)
                    .inspect("notes.save-error"),
            );
        }

        if let Some(notice) = notice {
            document = document.child(
                div()
                    .px_8()
                    .py_3()
                    .bg(rgb(0xeff3e9))
                    .text_color(rgb(GREEN))
                    .flex()
                    .justify_between()
                    .gap_3()
                    .child(notice)
                    .child(
                        div()
                            .id("dismiss-recovery")
                            .cursor_pointer()
                            .child("Dismiss")
                            .on_click(cx.listener(|this, _, _, _| this.notebook.dismiss_notice())),
                    )
                    .inspect("notes.recovery"),
            );
        }

        if self.help {
            document = document.child(div().mx_8().mt_5().p_5().rounded_lg().bg(rgb(0xf0f1e9)).line_height(px(24.))
                .child("A quiet place to think. Fieldnotes saves notes on this device, without an account. Edits get a recovery checkpoint, then an automatic save after a short pause. Use Save now before unplugging a drive or copying the library. Archive keeps notes until you restore them.")
                .child(div().mt_2().text_color(rgb(GREEN)).child("Keyboard: Cmd/Ctrl N · new note   /   F · search   /   S · save   /   Z · undo")));
        }

        if let Some(note) = selected {
            let id = note.id;

            document =
                document.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .px(px(54.))
                        .pt_10()
                        .pb_5()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .mb_5()
                                .child(div().text_size(px(11.)).text_color(rgb(MUTED)).child(
                                    format!(
                                        "{}  ·  PERSONAL NOTE",
                                        relative_date(note.modified_ms)
                                    ),
                                ))
                                .child(
                                    div()
                                        .flex()
                                        .gap_4()
                                        .text_size(px(11.))
                                        .text_color(rgb(GREEN))
                                        .child(
                                            div()
                                                .id("pin-note")
                                                .role(gpui::accesskit::Role::Button)
                                                .aria_label(if note.pinned {
                                                    "Unpin note"
                                                } else {
                                                    "Pin note"
                                                })
                                                .tab_index(0)
                                                .cursor_pointer()
                                                .child(if note.pinned {
                                                    "◆ Pinned"
                                                } else {
                                                    "◇ Pin note"
                                                })
                                                .on_click(cx.listener(move |this, _, _, _| {
                                                    let _ = this.notebook.pin(id);
                                                }))
                                                .inspect("notes.pin"),
                                        )
                                        .child(
                                            div()
                                                .id("archive-note")
                                                .role(gpui::accesskit::Role::Button)
                                                .aria_label(if note.archived {
                                                    "Restore note"
                                                } else {
                                                    "Archive note"
                                                })
                                                .tab_index(0)
                                                .cursor_pointer()
                                                .child(if note.archived {
                                                    "Restore"
                                                } else {
                                                    "Archive"
                                                })
                                                .on_click(cx.listener(move |this, _, _, _| {
                                                    let _ = this.notebook.archive(id);
                                                }))
                                                .inspect("notes.archive-note"),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .h(px(60.))
                                .flex_shrink_0()
                                .child(self.title.clone())
                                .inspect("notes.title"),
                        )
                        .child(div().w_12().h(px(2.)).mt_3().mb_7().bg(rgb(0xd6dece)))
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .child(self.body.clone())
                                .inspect("notes.body"),
                        )
                        .child(
                            div()
                                .flex()
                                .justify_between()
                                .items_center()
                                .pt_5()
                                .border_t_1()
                                .border_color(rgb(LINE))
                                .text_size(px(11.))
                                .text_color(rgb(MUTED))
                                .child(format!(
                                    "{} words   ·   {} characters",
                                    note.word_count(),
                                    note.body.chars().count()
                                ))
                                .child(
                                    div()
                                        .id("copy-note")
                                        .role(gpui::accesskit::Role::Button)
                                        .aria_label("Copy as Markdown")
                                        .tab_index(0)
                                        .cursor_pointer()
                                        .text_color(rgb(GREEN))
                                        .child(if self.copied {
                                            "Copied ✓"
                                        } else {
                                            "Copy as Markdown ↗"
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(library) = this.notebook.library()
                                                && let Some(note) = library
                                                    .notes
                                                    .iter()
                                                    .find(|note| Some(note.id) == library.selected)
                                            {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    format!(
                                                        "# {}\n\n{}\n",
                                                        note.display_title(),
                                                        note.body
                                                    ),
                                                ));
                                                this.copied = true;
                                                cx.notify();
                                            }
                                        }))
                                        .inspect("notes.copy"),
                                ),
                        ),
                );
        } else {
            document = document.child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(rgb(MUTED))
                    .child("Choose a note, or start with something new."),
            );
        }

        root.child(document).inspect("notes.workspace")
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
