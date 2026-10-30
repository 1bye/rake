//! Visible typed storage demo with a Chrome Application/Console connection.
//! Default GPUI builds show a window; --headless or no-default-features serve only IPC.

#[cfg(unix)]
mod native {
    use rake::{
        control::{CommandError, local::LocalServer},
        router::{NoRoute, Router, RouterConfig},
        services::ServiceRegistry,
        storage::{
            CacheConfig, FileBackend, MemoryBackend, Store,
            devtools::{Area, Storage},
        },
    };
    use std::path::PathBuf;

    #[rake::data(schema)]
    #[serde(rename_all = "lowercase")]
    enum Theme {
        Dark,
        Light,
        System,
    }

    #[rake::data(schema)]
    #[serde(deny_unknown_fields)]
    struct Preferences {
        theme: Theme,
        tab_width: u8,
    }

    #[rake::data(schema)]
    #[derive(rake::Command)]
    #[command(id = "test.storage", output = Option<Preferences>, capability = Headless)]
    #[serde(deny_unknown_fields)]
    struct Change {
        #[serde(default)]
        theme: Option<Theme>,
    }

    struct Demo {
        #[cfg(feature = "gpui")]
        preferences: Store<Preferences>,
        router: Router<NoRoute>,
        server: LocalServer,
    }

    pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
        let (demo, headless) = futures::executor::block_on(prepare())?;
        #[cfg(feature = "gpui")]
        if !headless {
            return window::run(demo);
        }
        #[cfg(not(feature = "gpui"))]
        let _ = headless;
        futures::executor::block_on(demo.serve())
    }

    async fn prepare() -> Result<(Demo, bool), Box<dyn std::error::Error>> {
        let mut arguments = std::env::args().skip(1);
        let mut data = None;
        let mut control = None;
        let mut headless = false;
        while let Some(argument) = arguments.next() {
            if argument == "--headless" {
                headless = true;
                continue;
            }
            let value = PathBuf::from(arguments.next().ok_or("expected directory argument")?);
            match argument.as_str() {
                "--data" => data = Some(value),
                "--control" => control = Some(value),
                _ => return Err("use --data DIRECTORY --control DIRECTORY".into()),
            }
        }
        let data = data.ok_or("--data DIRECTORY required")?;
        let control = control.ok_or("--control DIRECTORY required")?;
        let preferences = Store::<Preferences>::new(FileBackend::open(data).await?)
            .with_cache(CacheConfig::new(32, 128 * 1024)?);
        if preferences.get("editor").await?.is_none() {
            preferences
                .set(
                    "editor",
                    &Preferences {
                        theme: Theme::System,
                        tab_width: 4,
                    },
                )
                .await?;
        }
        let session = Store::<String>::new(MemoryBackend::new());
        session
            .set("example", &"Typed storage example".into())
            .await?;
        let inspection = Storage::new()
            .read_write("preferences", Area::Local, preferences.clone())?
            .read_only("session", Area::Session, session)?;
        let services = ServiceRegistry::new()
            .with(preferences.clone())?
            .with(inspection)?;
        let router = Router::<NoRoute>::with_services(RouterConfig::default(), services);
        let client = router.client();
        client
            .commands()
            .register::<Change, _, _>(|change, context| async move {
                let store = context
                    .services
                    .get::<Store<Preferences>>()
                    .map_err(|error| CommandError::Failed(error.to_string()))?;
                if let Some(theme) = change.theme {
                    let mut preferences =
                        store
                            .get("editor")
                            .await
                            .map_err(failed)?
                            .unwrap_or(Preferences {
                                theme: Theme::System,
                                tab_width: 4,
                            });
                    preferences.theme = theme;
                    store.set("editor", &preferences).await.map_err(failed)?;
                }
                store.get("editor").await.map_err(failed)
            })?;
        let server = LocalServer::bind(&client, "rake-storage-example", &control)?;
        Ok((
            Demo {
                #[cfg(feature = "gpui")]
                preferences,
                router,
                server,
            },
            headless,
        ))
    }

    impl Demo {
        async fn serve(self) -> Result<(), Box<dyn std::error::Error>> {
            let client = self.router.client();
            println!(
                "{}",
                serde_json::json!({"event":"storage_ready","runtime_id":client.info().runtime_id})
            );
            match futures::future::select(
                Box::pin(self.router.run()),
                Box::pin(self.server.run(client)),
            )
            .await
            {
                futures::future::Either::Left((result, _)) => result?,
                futures::future::Either::Right((result, _)) => result?,
            }
            Ok(())
        }
    }
    fn failed(error: rake::storage::StorageError) -> CommandError {
        CommandError::Failed(error.to_string())
    }

    #[cfg(feature = "gpui")]
    mod window {
        use super::*;
        use rake::gpui::{Context, Task, Window, div, prelude::*, rgb};

        pub(super) fn run(demo: Demo) -> Result<(), Box<dyn std::error::Error>> {
            rake::App::new("com.example.rake.storage")
                .root(move |_, _, cx| cx.new(|cx| StorageWindow::new(demo, cx)).into())
                .run()?;
            Ok(())
        }

        struct StorageWindow {
            preferences: Store<Preferences>,
            settings: String,
            dark: bool,
            status: String,
            saving: bool,
            write: Option<Task<()>>,
            _driver: Task<()>,
            _refresh: Task<()>,
        }

        impl StorageWindow {
            fn new(demo: Demo, cx: &mut Context<Self>) -> Self {
                let preferences = demo.preferences.clone();
                let observe = preferences.clone();
                let driver = cx.spawn(async move |_, _| {
                    if let Err(error) = demo.serve().await {
                        rake::error!(%error, "Storage demo host stopped");
                    }
                });
                // The demo explicitly refreshes its display; the storage API adds
                // no implicit UI polling to production applications.
                let refresh = cx.spawn(async move |view, cx| {
                    loop {
                        let (settings, dark) = match observe.get("editor").await {
                            Ok(Some(value)) => (
                                serde_json::to_string_pretty(&value)
                                    .unwrap_or_else(|error| error.to_string()),
                                matches!(value.theme, Theme::Dark),
                            ),
                            Ok(None) => ("The editor preferences were removed.".into(), false),
                            Err(error) => (error.to_string(), false),
                        };
                        if view
                            .update(cx, |view, cx| {
                                if view.settings != settings || view.dark != dark {
                                    view.settings = settings;
                                    view.dark = dark;
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                        rake::tasks::sleep(std::time::Duration::from_millis(250)).await;
                    }
                });
                Self {
                    preferences,
                    settings: "Loading settings…".into(),
                    dark: false,
                    status: "Changes are saved to the example's file store.".into(),
                    saving: false,
                    write: None,
                    _driver: driver,
                    _refresh: refresh,
                }
            }
            fn set_theme(&mut self, dark: bool, cx: &mut Context<Self>) {
                if self.saving {
                    return;
                }
                self.saving = true;
                let store = self.preferences.clone();
                self.write = Some(cx.spawn(async move |view, cx| {
                    let result = async {
                        let mut value = store.get("editor").await?.unwrap_or(Preferences {
                            theme: Theme::System,
                            tab_width: 4,
                        });
                        value.theme = if dark { Theme::Dark } else { Theme::Light };
                        store.set("editor", &value).await
                    }
                    .await;
                    let _ = view.update(cx, |view, cx| {
                        view.saving = false;
                        view.status = result
                            .map(|()| "Saved. Chrome's storage table updates automatically.".into())
                            .unwrap_or_else(|error| error.to_string());
                        cx.notify();
                    });
                }));
                cx.notify();
            }
        }

        impl Render for StorageWindow {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .items_center()
                    .gap_4()
                    .bg(rgb(if self.dark { 0x18232b } else { 0xf5f5f4 }))
                    .text_color(rgb(if self.dark { 0xf0f4e8 } else { 0x18232b }))
                    .child(div().text_3xl().child("Rake Storage"))
                    .child("Edit preferences/editor in Chrome's Application → Local storage.")
                    .child(
                        div()
                            .font_family("monospace")
                            .text_xl()
                            .child(self.settings.clone()),
                    )
                    .child(
                        div().flex().gap_4().children(
                            [("Use light theme", false), ("Use dark theme", true)]
                                .into_iter()
                                .map(|(label, dark)| {
                                    div()
                                        .id(if dark { "dark-theme" } else { "light-theme" })
                                        .px_4()
                                        .py_2()
                                        .rounded_md()
                                        .bg(rgb(0x365849))
                                        .text_color(rgb(0xffffff))
                                        .cursor_pointer()
                                        .child(label)
                                        .on_click(cx.listener(move |view, _, _, cx| {
                                            view.set_theme(dark, cx)
                                        }))
                                }),
                        ),
                    )
                    .child(self.status.clone())
            }
        }
    }
}

#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    native::run()
}

#[cfg(not(unix))]
fn main() {
    eprintln!("This transport example requires Unix; typed memory storage is portable.");
}
