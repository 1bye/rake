# Rake

Rake is a framework that provides an out-of-the-box experience with
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) in the agentic era.
Build desktop and browser apps from one Rust codebase, with tools that let you and your
AI agents inspect and control the running app.

> **Alpha.** APIs can change between revisions. See [what's supported](docs/support.md).

## What you get

- **One app, two targets.** One application factory for native and WebAssembly, with
  direct GPUI access through `rake::gpui`.
- **App structure.** Typed routes, independent tabs and histories, native multi-window
  support, and save/discard/cancel guards.
- **Managed async work.** Route loaders, retries, and tasks owned by tabs or the app.
- **Shared services.** HTTP, storage, diagnostics, files, clipboard and preferences
  behind one `rake` dependency.
- **Agent tooling.** Commands with JSON schemas, JSON CLI output, element inspection and
  Chrome DevTools.
- **Development and shipping.** Watched builds, checkpointed reloads, packaging,
  signing and update controllers.
- **UI options.** The `view!` syntax on [GPUI Base](docs/gpui-kit.md), optional
  GPUI Kit components (native only), and [AppKit controls](crates/rake-native/README.md).

## Quick start

Rake needs Rust 1.98.0, native development tools and Python 3.11+. Install the CLI
and create a desktop/browser app:

```sh
cargo install --git https://github.com/1bye/rake --locked rake-cli
rake new hello-rake
cd hello-rake
rake dev
```

The generated app has two pages, a persistent layout, a theme and an agent-callable
command. For the browser, use `rake dev --target web` with a WebGPU-capable browser
and the matching `wasm-bindgen-cli` (the builder prints the install command).
The first dev run creates `Cargo.lock`; commit it. `rake new` pins Rake to the
exact revision the CLI was built from.

UI uses `view!`. A complete Hello window looks like this:

```rust
use rake::gpui::{Context, Window, prelude::*};
use rake::prelude::*;

struct Hello;

impl Render for Hello {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        rake::view! { 
          <main class="size-full p-6">
            <h1>"Hello, Rake!"</h1>
          </main> 
        }
    }
}

#[rake::app]
fn app() -> App {
    App::new("com.example.hello-rake")
        .root(|_, _, cx| cx.new(|_| Hello).into())
}
```

To add Rake to an existing crate, copy the pinned dependency and parser patch from
a generated `Cargo.toml`, then use the Hello entry above. `view!`, themes and style
presets are included by default; Kit and DevTools remain optional.

[Getting started](docs/getting-started.md) covers browser prerequisites, local
Rake development and agents. On macOS, check that `which rake` selects Cargo's CLI
rather than the system Ruby tool.

## Try the examples

From a clone of this repository:

| Command | Shows |
| --- | --- |
| `cargo run -p rake-notes` | Fieldnotes, the [reference app](docs/notes.md): search, autosave, crash recovery |
| `cargo run -p rake-foundation` | App setup, HTTP and diagnostics |
| `cargo run -p rake-routing` | Tabs, histories, guards and managed work |
| `cargo run -p rake-view` | The `view!` syntax |
| `cargo run -p rake-desktop-example` | Pages, layouts, loading and native updates |
| `cargo run -p rake-files` | Files, clipboard and preferences |
| `cargo run -p rake-kit-example --features kit,devtools` | GPUI Kit in routed tabs (native) |
| `cargo run -p rake --example native --features native-components` | AppKit controls (macOS) |

## Documentation

Start at the [documentation index](docs/README.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Licensed under either the [MIT License](LICENSE-MIT) or the
[Apache License, Version 2.0](LICENSE-APACHE), at your option.
