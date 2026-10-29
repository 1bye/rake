//! Typed routing with independently retained drafts and structured save/discard decisions.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use rake::gpui::prelude::*;
use rake::gpui::{AnyView, Context, Window, div, rgb};
use rake::prelude::*;
use rake::router::MountError;

#[derive(Clone, Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "document", path = "/documents/{id}")]
    Document(String),
}

#[rake::data]
struct DraftState {
    edits: u32,
}

impl ReloadState for DraftState {
    const SCHEMA: &'static str = "routing.draft";
    const VERSION: u32 = 1;
}

#[derive(Default)]
struct AppRoutes {
    export: Rc<RefCell<Option<TaskHandle>>>,
}

struct Tick {
    run: rake::router::RunId,
    preview: bool,
}

impl RouteViews<Destination> for AppRoutes {
    fn mount(
        &self,
        destination: &Destination,
        page: PageContext<Destination>,
        _window: &mut Window,
        cx: &mut rake::gpui::App,
    ) -> Result<AnyView, MountError> {
        let title = match destination {
            Destination::Home => "Home".into(),
            Destination::Document(id) => format!("Document {id}"),
        };

        let entry = page.entry();

        let restored = entry
            .reload_state::<DraftState>()
            .map_err(|error| MountError(error.to_string()))?;

        let edits = Rc::new(Cell::new(restored.map_or(0, |state| state.edits)));
        let captured = edits.clone();

        entry
            .on_reload(move || {
                DraftState {
                    edits: captured.get(),
                }
                .checkpoint()
            })
            .map_err(|error| MountError(error.to_string()))?;

        entry
            .clone()
            .on_dispose(move |context| {
                let entry = entry.clone();

                async move {
                    if !context.dirty {
                        return GuardResult::Allow;
                    }

                    GuardResult::Ask(DecisionRequest {
                        message: "Save this document before leaving it?".into(),
                        choices: vec![
                            DecisionChoice::proceed("save", "Save").prepare(move || {
                                let entry = entry.clone();

                                async move {
                                    entry.set_dirty(false).map_err(|error| {
                                        rake::router::GuardReason(error.to_string())
                                    })
                                }
                            }),
                            DecisionChoice::proceed("discard", "Discard"),
                            DecisionChoice::cancel("cancel", "Cancel"),
                        ],
                    })
                }
            })
            .map_err(|error| MountError(error.to_string()))?;

        let view = cx.new(|cx| Screen {
            navigation_focus: cx.focus_handle(),
            title,
            page: page.clone(),
            edits,
            ticks: 0,
            previews: 0,
            refresh_run: None,
            preview_run: None,
            download: None,
            task_error: None,
            command_sequence: 0,
            export: self.export.clone(),
        });

        let screen = view.downgrade();
        let context = cx.to_async();

        page.on_event(move |tick: Tick| {
            screen
                .update(&mut context.clone(), |screen, cx| {
                    if tick.preview {
                        screen.previews += 1;
                        screen.preview_run = Some(tick.run);
                    } else {
                        screen.ticks += 1;
                        screen.refresh_run = Some(tick.run);
                    }

                    cx.notify();
                })
                .map_err(|error| TaskError::Failed(error.to_string()))
        })
        .map_err(|error| MountError(error.to_string()))?;
        page.activity("refresh", |run| async move {
            loop {
                rake::tasks::sleep(std::time::Duration::from_millis(500)).await;
                run.publish(Tick {
                    run: run.id(),
                    preview: false,
                })?;
            }
        })
        .map_err(|error| MountError(error.to_string()))?;
        page.while_visible("preview", |run| async move {
            loop {
                rake::tasks::sleep(std::time::Duration::from_millis(500)).await;
                run.publish(Tick {
                    run: run.id(),
                    preview: true,
                })?;
            }
        })
        .map_err(|error| MountError(error.to_string()))?;

        Ok(view.into())
    }
}

struct Screen {
    navigation_focus: rake::gpui::FocusHandle,
    title: String,
    page: PageContext<Destination>,
    edits: Rc<Cell<u32>>,
    ticks: u64,
    previews: u64,
    refresh_run: Option<rake::router::RunId>,
    preview_run: Option<rake::router::RunId>,
    download: Option<TaskHandle>,
    task_error: Option<String>,
    command_sequence: u64,
    export: Rc<RefCell<Option<TaskHandle>>>,
}

impl Render for Screen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let navigator = self.page.navigator();
        let new_window = navigator.clone();
        let close_window = navigator.clone();
        let open_a = navigator.clone();
        let open_b = navigator.clone();
        let hidden = navigator.clone();
        let home = navigator.clone();
        let reveal = navigator.clone();
        let back = self.page.tab();
        let forward = self.page.tab();

        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .items_center()
            .gap_4()
            .child(div().text_3xl().child(self.title.clone()))
            .child(format!(
                "{} / {}",
                self.page.window_id(),
                self.page
                    .entry()
                    .id()
                    .map(|id| id.to_string())
                    .unwrap_or_default()
            ))
            .when(cfg!(not(target_family = "wasm")), |view| {
                view.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            div()
                                .id("new-window")
                                .p_2()
                                .bg(rgb(0x29424b))
                                .hover(|style| style.bg(rgb(0x365849)))
                                .cursor_pointer()
                                .child("Open A in new window")
                                .on_click(move |_, _, _| {
                                    drop(new_window.open(
                                        Destination::Document("A".into()),
                                        OpenTarget::NewWindow,
                                    ));
                                })
                                .inspect("routing.new-window"),
                        )
                        .child(
                            div()
                                .id("close-window")
                                .p_2()
                                .bg(rgb(0x29424b))
                                .cursor_pointer()
                                .child("Close this window")
                                .on_click(move |_, _, _| {
                                    drop(close_window.close_window());
                                })
                                .inspect("routing.close-window"),
                        ),
                )
            })
            .child(
                div()
                    .id("command-echo")
                    .track_focus(&self.navigation_focus)
                    .px_3()
                    .py_2()
                    .bg(rgb(0x365849))
                    .cursor_pointer()
                    .child("Run shared echo command")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.command_sequence += 1;
                        this.task_error = this
                            .page
                            .entry()
                            .id()
                            .map_err(|error| error.to_string())
                            .and_then(|entry| {
                                let key = format!("echo-{entry}-{}", this.command_sequence);

                                this.page
                                    .dispatch(
                                        Echo {
                                            message: this.title.clone(),
                                        },
                                        key,
                                    )
                                    .map_err(|error| error.to_string())
                            })
                            .err();
                        cx.notify();
                    }))
                    .inspect("routing.command-echo")
                    .focus_handle(&self.navigation_focus),
            )
            .child(format!("Draft edits: {}", self.edits.get()))
            .child(format!(
                "Refresh ticks: {} ({:?}) · Visible preview: {} ({:?})",
                self.ticks, self.refresh_run, self.previews, self.preview_run
            ))
            .child(format!(
                "Download: {:?} · App export: {:?}",
                self.download
                    .as_ref()
                    .and_then(|task| task.snapshot().ok())
                    .map(|task| task.state),
                self.export
                    .borrow()
                    .as_ref()
                    .and_then(|task| task.snapshot().ok())
                    .map(|task| task.state)
            ))
            .child("Open the same document twice, edit one, then switch tabs or use Back.")
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("foreground")
                            .px_3()
                            .py_2()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Foreground")
                            .on_click(cx.listener(|this, _, _, _| {
                                drop(this.page.tab().set_mode(TabMode::Foreground));
                            }))
                            .inspect("routing.foreground"),
                    )
                    .child(
                        div()
                            .id("live")
                            .px_3()
                            .py_2()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Live")
                            .on_click(cx.listener(|this, _, _, _| {
                                drop(this.page.tab().set_mode(TabMode::Live));
                            }))
                            .inspect("routing.live"),
                    )
                    .child(
                        div()
                            .id("download")
                            .px_3()
                            .py_2()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Start tab download")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let submitted = this.page.tab().tasks().spawn("download", async {
                                    rake::tasks::sleep(std::time::Duration::from_secs(8)).await;

                                    Ok(())
                                });

                                match submitted {
                                    Ok(task) => {
                                        this.download = Some(task);
                                        this.task_error = None;
                                    }
                                    Err(error) => this.task_error = Some(error.to_string()),
                                }

                                cx.notify();
                            }))
                            .inspect("routing.download"),
                    )
                    .child(
                        div()
                            .id("cancel-download")
                            .px_3()
                            .py_2()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Cancel download")
                            .on_click(cx.listener(|this, _, _, _| {
                                if let Some(task) = &this.download {
                                    let _ = task.cancel();
                                }
                            }))
                            .inspect("routing.cancel-download"),
                    )
                    .child(
                        div()
                            .id("export")
                            .px_3()
                            .py_2()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Start app export")
                            .on_click(cx.listener(|this, _, _, cx| {
                                let submitted = this.page.app().tasks().spawn("export", async {
                                    rake::tasks::sleep(std::time::Duration::from_secs(8)).await;

                                    Ok(())
                                });

                                match submitted {
                                    Ok(task) => {
                                        *this.export.borrow_mut() = Some(task);
                                        this.task_error = None;
                                    }
                                    Err(error) => this.task_error = Some(error.to_string()),
                                }

                                cx.notify();
                            }))
                            .inspect("routing.export"),
                    ),
            )
            .children(self.task_error.clone())
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("edit")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x365849))
                            .cursor_pointer()
                            .child("Edit draft")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.edits.set(this.edits.get() + 1);
                                let _ = this.page.entry().set_dirty(true);

                                cx.notify();
                            }))
                            .inspect("routing.edit"),
                    )
                    .child(
                        div()
                            .id("save")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x365849))
                            .cursor_pointer()
                            .child("Save")
                            .on_click(cx.listener(|this, _, _, _| {
                                let _ = this.page.entry().set_dirty(false);
                            }))
                            .inspect("routing.save"),
                    )
                    .child(
                        div()
                            .id("open-a")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Open A")
                            .on_click(move |_, _, _| {
                                drop(
                                    open_a.open(
                                        Destination::Document("A".into()),
                                        OpenTarget::NewTab,
                                    ),
                                );
                            })
                            .inspect("routing.open-a"),
                    )
                    .child(
                        div()
                            .id("open-b")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Open B")
                            .on_click(move |_, _, _| {
                                drop(
                                    open_b.open(
                                        Destination::Document("B".into()),
                                        OpenTarget::NewTab,
                                    ),
                                );
                            })
                            .inspect("routing.open-b"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("home")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Home in this tab")
                            .on_click(move |_, _, _| {
                                drop(home.open(Destination::Home, OpenTarget::CurrentTab));
                            })
                            .inspect("routing.home"),
                    )
                    .child(
                        div()
                            .id("hidden")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Open B without selecting")
                            .on_click(move |_, _, _| {
                                drop(
                                    hidden.open_with(
                                        Destination::Document("B".into()),
                                        OpenOptions::new_tab()
                                            .activation(Activation::PreserveSelection),
                                    ),
                                );
                            })
                            .inspect("routing.hidden"),
                    )
                    .child(
                        div()
                            .id("reveal")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Reveal A")
                            .on_click(move |_, _, _| {
                                drop(reveal.reveal(
                                    Destination::Document("A".into()),
                                    RevealScope::Window,
                                ));
                            })
                            .inspect("routing.reveal"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .id("back")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Back in this tab")
                            .on_click(move |_, _, _| {
                                drop(back.back());
                            })
                            .inspect("routing.back"),
                    )
                    .child(
                        div()
                            .id("forward")
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(rgb(0x29424b))
                            .cursor_pointer()
                            .child("Forward in this tab")
                            .on_click(move |_, _, _| {
                                drop(forward.forward());
                            })
                            .inspect("routing.forward"),
                    ),
            )
            .inspect("routing.screen")
    }
}

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "demo.echo", output = String, capability = Headless)]
#[serde(deny_unknown_fields)]
struct Echo {
    #[serde(default = "default_echo")]
    message: String,
}

fn default_echo() -> String {
    "Hello from the inspector".into()
}

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "demo.fetch", output = FetchResult, capability = Headless)]
#[serde(deny_unknown_fields)]
struct Fetch {
    url: String,
}

#[rake::data(schema)]
struct FetchResult {
    status: u16,
    bytes: usize,
}

#[rake::app]
fn app() -> App<Destination> {
    let app = App::new("com.example.rake.routing")
        .control(rake::control::ControlConfig::FromEnvironment)
        .routes(AppRoutes::default())
        .web(WebConfig::new().routing(if cfg!(feature = "hash-routing") {
            WebRouting::Hash
        } else {
            WebRouting::Path
        }));

    let http = app.services().expect("example HTTP services").http();
    let commands = Commands::new();

    commands
        .register::<Echo, _, _>(|command, _| async move {
            rake::info!(message = %command.message, "Inspector echo");

            Ok(command.message)
        })
        .expect("echo registration");
    commands
        .register::<Fetch, _, _>(move |command, context| {
            let http = http.clone();

            async move {
                let response = http
                    .get(&command.url)
                    .cancellation(context.cancellation)
                    .send()
                    .await
                    .map_err(|error| CommandError::Failed(error.to_string()))?;

                let status = response.status();

                let bytes = response
                    .bytes()
                    .await
                    .map_err(|error| CommandError::Failed(error.to_string()))?;

                rake::info!(status, bytes = bytes.len(), "Inspector request completed");

                Ok(FetchResult {
                    status,
                    bytes: bytes.len(),
                })
            }
        })
        .expect("fetch registration");

    app.commands(commands)
}
