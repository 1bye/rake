# Getting started

This takes you from nothing to a running app, then adds the CLI, the browser and agents.

## Prerequisites

- Rust 1.98.0. Inside this repository, `rust-toolchain.toml` selects it for you.
- On macOS, Apple's development tools.
- For the CLI, Python 3.11 or newer.
- For the browser, a WebGPU-capable browser.

## Your first app

The [README quick start](../README.md#quick-start) creates a window in one file. To explore
further, clone this repository and run an example:

```sh
cargo run -p rake-foundation --locked
```

These make good starting points:

- [Foundation](../examples/foundation/README.md): a standalone window with loading and retry.
- [Desktop](../examples/desktop/README.md): pages, layouts and native updates.
- [Text Toolkit](../examples/files/README.md): files, clipboard and preferences with no routes.
- [Fieldnotes](notes.md): a complete app.

## Install the CLI

The CLI gives you watched builds, reloads and Chrome DevTools:

```sh
cargo install --path crates/rake-cli --locked
rake dev --package rake-notes --cdp
```

On macOS, Ruby also provides `/usr/bin/rake`. Check that `which rake` shows the CLI you just
installed, normally `~/.cargo/bin/rake`. If it shows Ruby's, put Cargo first:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
rehash # zsh; use hash -r in bash
which rake
```

See [development and reload](development.md) for build options, and
[Chrome DevTools](devtools.md) for connecting the inspector.

## Run in the browser

```sh
rake dev --target web --package rake-notes --cdp
```

This needs the `wasm32-unknown-unknown` target and a `wasm-bindgen-cli` matching `Cargo.lock`.
The builder prints the install command if yours is missing. Your own app also needs a library
entry point and configuration, which the [Foundation example](../examples/foundation/README.md)
shows. Rake's browser apps draw to a GPUI canvas rather than HTML, and GPUI Kit is native only.
See [browser apps](web-applications.md).

## Connect an agent

Agents can discover your app's commands and schemas, run them and read the results. `rake dev`
prints a session directory. In another terminal, point `RAKE_CONTROL_DIR` at its `control`
subdirectory, using your own session name:

```sh
export RAKE_CONTROL_DIR=/tmp/rake-dev-EXAMPLE/control
rake instances --json
rake ctl commands list --json
rake ctl commands run notes.list --args null --json
```

Without this variable, the CLI searches its default directory and can return `[]` even while
your app is running.

For more, see [runtime control](runtime-control.md) and the
[CDP agent client](cdp-agents.md), which handles UI input, waits and screenshots.

## Next steps

- [Application shapes](application.md) explains how an app fits together.
- [Routing](routing.md) adds pages, tabs and guards.
- [View syntax](experimental-view.md) is a shorter way to write GPUI layouts.
- [The documentation index](README.md) lists every guide.
