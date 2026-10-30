//! Runnable typed storage service with a Chrome Application/Console connection.
//! Run with --no-default-features --features devtools --example storage.

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

    pub(super) async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let mut arguments = std::env::args().skip(1);
        let mut data = None;
        let mut control = None;
        while let Some(argument) = arguments.next() {
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
        let services = ServiceRegistry::new().with(preferences)?.with(inspection)?;
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
        println!(
            "{}",
            serde_json::json!({"event":"storage_ready","runtime_id":client.info().runtime_id})
        );
        match futures::future::select(Box::pin(router.run()), Box::pin(server.run(client))).await {
            futures::future::Either::Left((result, _)) => result?,
            futures::future::Either::Right((result, _)) => result?,
        }
        Ok(())
    }
    fn failed(error: rake::storage::StorageError) -> CommandError {
        CommandError::Failed(error.to_string())
    }
}

#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    futures::executor::block_on(native::run())
}

#[cfg(not(unix))]
fn main() {
    eprintln!("This transport example requires Unix; typed memory storage is portable.");
}
