# Typed application services

Standard services also expose shared [platform services](platform-services.md) via
`services.platform()`: file import/export, clipboard text, namespaced preferences,
URL opening and host activations. They are available in roots, pages and commands.

Compose concrete services once at the application boundary. Routed views and
typed command handlers resolve the same `Rc<T>` without global variables or
capturing a second copy in a command registry.

```rust,ignore
let app = rake::App::new("dev.example.notes")
    .service(Notebook::new(repository))?
    .routes(Views)
    .commands(Notebook::commands()?);

// RouteViews::mount
let notebook = page.services().get::<Notebook>()?;

// A registered typed command handler
let notebook = context.services.get::<Notebook>()?;
```

`ServiceRegistry::with` returns a new immutable composition. Cloning a registry
shares its concrete values; registering another type leaves earlier snapshots
unchanged. Duplicate registrations and missing types return `ServiceError`.
Services need `'static`, with no `Send`/`Sync` requirement. A service owns its
mutability, cancellation and external resources. Do not retain UI borrows across
an asynchronous call.

`App::services()` combines the registry with Rake's HTTP transport and diagnostics.
`Services::http()` and `Services::diagnostics()` retain their existing behavior.
Headless hosts use `Router::with_services(config, registry)`; its runtime client
passes that registry to every `CommandContext`. Register handlers on
`router.client().commands()` as usual. Composition is not serialized into reload
checkpoints: the application recreates services, then restores compatible owned
data using entry reload hooks.

The [Fieldnotes reference app](notes.md) composes a notebook service, gives its
autosave loop to a managed Live activity, and flushes it from a disposal guard.
Its native filesystem worker receives only owned serializable data, never the
local service or a view entity.
