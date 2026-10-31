# Rake

Rake is a framework for GPUI in the agentic era.

Build native and browser apps in Rust, with tools that let developers and agents
inspect and control the running application. The GUI, CLI and devtools share
application commands, operation results and lifecycle guards.

**Status:** unpublished prototype.

## What Rake provides

- **GPUI across targets.** One application factory for native and browser
  WebAssembly, with direct GPUI access through `rake::gpui`.
- **Application structure.** Typed routes, independent tabs and histories, native
  multi-window support, and save/discard/cancel guards.
- **Managed async work.** Tasks owned by tabs or the application, with cancellation
  and visibility-aware activities.
- **Shared services.** Typed service composition, HTTP, storage and structured
  diagnostics behind one `rake` dependency.
- **Tools for agents.** Discoverable commands with argument/result schemas, JSON
  CLI output, element inspection and Chrome DevTools integration.
- **Development workflow.** Watched builds, checkpointed reloads, native browser
  preview and native/Web packaging.

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
| Application model | [Routing](docs/routing.md), [managed work](docs/managed-work.md), [guarded quit](docs/guarded-quit.md) |
| Data and services | [Service composition](docs/services.md), [typed storage](docs/storage.md) |
| UI | [Experimental view syntax](docs/experimental-view.md), [element inspection](docs/element-inspection.md) |
| Developer tools | [Runtime control](docs/runtime-control.md), [Chrome DevTools](docs/devtools.md), [native browser preview](docs/dev-preview.md) |
| Build and ship | [Development and reload](docs/development.md), [packaging](docs/packaging.md) |
| Reference | [Fieldnotes app](docs/notes.md), [API design](docs/api-design.md) |

## Status

Runtime validation currently covers macOS and Chromium. Other native platforms
and browsers need validation. Native applications support multiple windows;
browser applications use one host. Native control transport requires Unix.

Native reload currently uses checkpointed restarts; code replacement awaits an
external hot reloader. Installers, notarization and durable sessions remain
future work. See the individual guides for feature limits.

## Verify

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The [CI workflow](.github/workflows/check.yml) contains the full native, Wasm,
devtools and documentation checks.
