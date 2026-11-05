# Rake

Rake is a framework for GPUI in the agentic era.

Build native and browser apps in Rust, with tools that let developers and agents
inspect and control the running application. The GUI, CLI and devtools share
application commands, operation results and lifecycle guards.

**Status:** unpublished alpha candidate. See the [support contract and release gates](docs/release-readiness.md) and [release qualification](docs/qualification.md).

## What Rake provides

- **GPUI across targets.** One application factory for native and browser
  WebAssembly, with direct GPUI access through `rake::gpui`.
- **Application structure.** Typed routes, independent tabs and histories, native
  multi-window support, and save/discard/cancel guards.
- **Managed async work.** Route loaders, retryable data, initiated user actions, and
  tasks owned by tabs or the application, with cancellation and visibility-aware activities.
- **Shared services.** Typed service composition, HTTP, storage and structured
  diagnostics behind one `rake` dependency.
- **Tools for agents.** Discoverable commands with argument/result schemas, JSON
  CLI output, element inspection and Chrome DevTools integration.
- **Development workflow.** Watched builds, checkpointed reloads, native browser
  preview and native/Web packaging.
- **Distribution and updates.** Icons, bundled dependencies/resources, desktop metadata,
  signing, publication and shared native update controllers.
- **Platform services.** Shared files, clipboard, preferences and URL activation, with
  owned browser workers, offline bundles and guarded deployment updates.
- **Optional native components.** AppKit buttons, segmented controls and custom
  native views in GPUI layouts through the `native-components` feature. See
  [rake-native](crates/rake-native/README.md) for the macOS adapter and its boundaries.

## Tiny examples

These use `rake::view!`, which needs the `experimental-view` feature. Plain GPUI
elements work everywhere through `rake::gpui`.

### A window

`#[rake::app]` generates the native and browser entry points from one factory:

```rust
use rake::gpui::{Context, Window, prelude::*};
use rake::prelude::*;

struct Counter {
    count: usize,
}

impl Render for Counter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <main class="size-full p-6 gap-4">
                <p>{format!("Count: {}", self.count)}</p>
                <button id="increment" on:click={cx.listener(|this, _, _, cx| {
                    this.count += 1;
                    cx.notify();
                })}>
                    "Increment"
                </button>
            </main>
        }
    }
}

#[rake::app]
fn app() -> App {
    App::new("com.example.counter")
        .root(|_, _, cx| cx.new(|_| Counter { count: 0 }).into())
}
```

### Async data

The loader belongs to the view: it redraws on completion and cancels with the view.

```rust
// When creating the view, with `root_with` providing `runtime`:
let text = runtime.load(cx, |load| async move {
    load.background_executor()
        .spawn(async { std::fs::read_to_string("welcome.txt") })
        .await
});

// In `render`:
let label = match self.text.state(cx) {
    LoadStatus::Ready(text) => text.clone(),
    LoadStatus::Failed(error) => error.to_string(),
    _ => "Loading…".into(),
};
```

### Typed routes

A destination with no registered page fails to compile:

```rust
#[derive(Clone, Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "document", path = "/documents/{id}")]
    Document(u64),
}

App::new("com.example.documents")
    .routes(rake::routes! {
        Destination::Home => Home,
        Destination::Document(..) => DocumentPage,
    })
    .initial(Destination::Home)
```

`Home` and `DocumentPage` implement `Page<Destination>`. See [routing](docs/routing.md).

### Driving a running app from the CLI

An AI agent can use the same terminal commands as you. `rake ctl` reads and controls
the application; set `RAKE_CONTROL_DIR` to the session path printed by `rake dev`:

```sh
rake instances --json
rake ctl commands list --json
rake ctl commands run notes.list --args null --json
rake ctl inspect tree --json
rake ctl logs list --json
```

`rake agent` works through Chrome DevTools Protocol (CDP), using the URL printed by
`rake dev --cdp`. It finds elements, sends real input and waits for the UI to update:

```sh
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"snapshot"}'
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"click","query":{"id":"increment"}}'
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"waitFor","query":{"text":"Count: 1"}}'
rake agent --url http://127.0.0.1:PORT/TOKEN '{"action":"screenshot","path":"/tmp/app.png"}'
```

Screenshots are native-only and need the adapter started with `--capture`. See
[Work with agents](#work-with-agents) and the [CDP agent client](docs/cdp-agents.md).

### Guarding quit

Standalone close, routed apps and update restarts all go through the same check:

```rust
App::new("com.example.editor")
    .quit_guard(|_| async { Err(rake::router::GuardReason("export is running".into())) })
```

See [guarded quit](docs/guarded-quit.md).

### Packaging

Describe the app in `Cargo.toml` and prepare its resources in `build.rs`:

```toml
[package.metadata.rake.bundle]
schema_version = 1
name = "My App"
identifier = "com.example.my-app"
icon = "assets/icon.svg"
resources = [{ from = "assets/help.txt", to = "help.txt" }]
```

```rust
// build.rs
fn main() {
    rake_build::build().expect("prepare desktop resources");
}
```

Check the prerequisites, preview the plan, then build. Desktop bundles default to DMG
on macOS, NSIS on Windows and AppImage on Linux:

```sh
rake doctor --distribution --package my-app --json
rake bundle --package my-app --plan --json
rake bundle --package my-app --json
rake bundle --target web --package my-app --base-path /my-app/ --routing path --json
```

See [desktop distribution](docs/distribution.md) and [packaging](docs/packaging.md).

### Updates

Updates are off by default. Enable them with an HTTPS root and your public keys:

```toml
[package.metadata.rake.bundle.updates]
enabled = true
channel = "stable"
base_url = "https://downloads.example.com/my-app"

[package.metadata.rake.bundle.updates.public_keys]
sparkle = "packaging/sparkle.pub"
velopack = "packaging/velopack.pub"
appimage = "packaging/appimage.pub"
```

```sh
rake update-key generate --private-key /secure/my-app-velopack.key --public-key packaging/velopack.pub
```

With the `updates` feature, a view binds to the shared controller. Restarting to
update runs your quit guards first:

```rust
// When creating the view, with `root_with` providing `runtime`:
let updates = runtime.bind_updates(cx);

// In `render`:
let can_check = self.updates.can_check(cx);
let can_restart = self.updates.can_restart(cx);

rake::view! {
    <div class="flex flex-row gap-3">
        <button disabled={!can_check}
            on:click={cx.listener(|this, _, _, cx| this.updates.check(cx))}>
            "Check for updates"
        </button>
        <button disabled={!can_restart}
            on:click={cx.listener(|this, _, _, cx| this.updates.restart(cx))}>
            "Restart to update"
        </button>
    </div>
}
```

### Publishing a release

Name where releases go and which targets each one must include:

```toml
[package.metadata.rake.bundle.release.publisher]
kind = "directory"
path = "release-site"
public_url = "https://downloads.example.com/my-app"

[package.metadata.rake.bundle.release.required]
aarch64-apple-darwin = ["zip", "dmg"]
x86_64-pc-windows-msvc = ["velopack"]
x86_64-unknown-linux-gnu = ["appimage", "deb", "rpm"]
```

The plan loads no signing credentials and publishes nothing. The release signs,
verifies and publishes, and a failure at any step prevents promotion:

```sh
rake release --plan --json
rake release --json
```

See [releasing native applications](docs/distribution-release.md).

## Try it

The repository pins Rust 1.95.0 and GPUI CE. From the workspace root, run
[Fieldnotes](docs/notes.md), the reference notes app with search, autosave and
crash recovery:

```sh
cargo run -p rake-notes
```

Other examples:

| Command | Demonstrates |
| --- | --- |
| `cargo run -p rake-foundation` | Application setup, HTTP and diagnostics |
| `cargo run -p rake-routing` | Tabs, histories, guards and managed work |
| `cargo run -p rake-view` | Experimental HTML-like `rake::view!` syntax |
| `cargo run -p rake-desktop-example` | Pages, persistent layouts, loading/retry and native updates |
| `cargo run -p rake-files` | File processing, clipboard/export, preferences and Web updates |
| `cargo run -p rake --example native --features native-components` | Real AppKit controls and native Liquid Glass buttons (macOS) |

For watched builds and Chrome DevTools, install the local CLI. Python 3.11+ is
required:

```sh
cargo install --path crates/rake-cli --locked
rake dev --package rake-notes --cdp
```

To run in the browser:

```sh
rake dev --target web --package rake-notes --cdp
```

Web builds need the `wasm32-unknown-unknown` target and a `wasm-bindgen-cli` version
matching Cargo.lock. The builder prints the install command when needed. Use a
WebGPU-capable browser. See [development and reload](docs/development.md) for
build options and [Chrome DevTools](docs/devtools.md) for connecting the inspector.

## Work with agents

Agents can discover application commands and their schemas, invoke actions and
read results through the CLI. In another terminal, set the session path printed
by `rake dev`:

```sh
export RAKE_CONTROL_DIR=/tmp/rake-dev-EXAMPLE/control
rake instances --json
rake ctl commands list --json
rake ctl commands run notes.list --args null --json
```

The same runtime exposes routes, tasks, pending decisions, logs and inspected UI
elements. Development control is opt-in; `rake dev` supplies the connection setup
for examples that enable it. See [runtime control](docs/runtime-control.md) for
application integration and the full command surface. For native UI automation,
see the [CDP agent client](docs/cdp-agents.md): semantic queries, input, condition
waits, screenshots and Rust source lookup.

## Documentation

| Topic | Guides |
| --- | --- |
| Application model | [Application shapes](docs/application.md), [routing](docs/routing.md), [managed work](docs/managed-work.md), [guarded quit](docs/guarded-quit.md) |
| Data and services | [Service composition](docs/services.md), [typed storage](docs/storage.md), [platform services](docs/platform-services.md) |
| UI | [Experimental view syntax](docs/experimental-view.md), [element inspection](docs/element-inspection.md) |
| Developer tools | [Runtime control](docs/runtime-control.md), [Chrome DevTools](docs/devtools.md), [native browser preview](docs/dev-preview.md) |
| Build and ship | [Development and reload](docs/development.md), [packaging](docs/packaging.md), [distribution](docs/distribution.md), [browser applications](docs/web-applications.md) |
| Reference | [Fieldnotes app](docs/notes.md), [API design](docs/api-design.md) |

## Status

Runtime validation includes macOS/Chromium, with focused Windows/Linux readiness
and installer workflows. Each release needs its own evidence; compilation, GUI
and installer checks establish different boundaries. Native applications support
multiple windows; browser applications use one host. Native control transport
requires Unix. Built-in host visibility measures macOS occlusion, Windows window
state and browser document visibility; Linux reports Unknown.

Native reload currently uses checkpointed restarts; code replacement awaits an
external hot reloader. Packaging, signing/notarization and publication are covered
by the distribution pipeline; durable sessions remain separate from development
checkpoints. Real-host screen-reader, display and sustained-workload qualification
remain beta release gates. See the individual guides for feature limits.

## Verify

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The [CI workflow](.github/workflows/check.yml) contains the full native, Wasm,
devtools and documentation checks.

The standalone [foundation example](examples/foundation/README.md) loads its JSON
fixture automatically and offers Reload / Retry. [Text Toolkit](examples/files/README.md)
exercises the common platform services without routes. Browser pickers/clipboard
requests start in user events, while Rake actions own observation and quit tracking.

The Rust spacing checker is pinned in Cargo metadata. Run
`python3 scripts/check-spacing.py` after formatting; it uses the project's separate
development-only nightly without changing the stable application toolchain.

## License

Licensed under either the [MIT License](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option.
