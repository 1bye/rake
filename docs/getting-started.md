# Getting started

This takes you from nothing to a running app, then adds the CLI, the browser and agents.

## Prerequisites

- Rust 1.98.0. Inside this repository, `rust-toolchain.toml` selects it for you.
- On macOS, Apple's development tools.
- For the browser, a WebGPU-capable browser.

## Create your first app

Install the CLI, then scaffold an app:

```sh
cargo install rake-app-cli --version 0.1.0-alpha.6 --locked
rake new my-app
cd my-app
rake dev
```

`rake new` embeds one small template: two typed pages written with `view!`, a
persistent layout, a semantic theme and a registered `app.greet` command. It
writes files offline, without running Cargo, and refuses existing non-empty
directories. Names must be portable Cargo package names (ASCII letters, digits,
hyphens and underscores, starting with a letter, at most 64 characters, excluding
reserved names). Paths and traversal are rejected. An existing empty directory
is accepted. Git is initialized only with `--git`; no commit is made.

The generated `rust-toolchain.toml` selects Rust 1.98.0 and the Wasm target. The
first `rake dev` or `rake build` resolves dependencies and creates `Cargo.lock`;
commit it. Subsequent builds use `--locked`.

The installed CLI pins the generated project to its tested `rake-app` SDK release,
which can differ from the CLI version. CLI alpha.6 uses SDK alpha.5. A CLI built
from a checkout pins that checkout revision instead. Pass
`--rake-rev REV` to use a full Git commit hash from a revision with the `rake-app`
package name.

When developing Rake itself, use a checkout path instead:

```sh
cargo install --path crates/rake-cli --locked
rake new my-app --rake-path /absolute/path/to/rake/crates/rake --git
```

The path is stored absolutely, so moving the generated app doesn't silently
change its dependency. Its README explains where to change the application ID.
For an existing crate, run `cargo add rake-app@0.1.0-alpha.5 --rename rake` and follow the [README Hello example](../README.md#quick-start).
Use the helper versions supported by that SDK when upgrading an existing app;
CLI, SDK and supporting-crate version numbers can differ.

On macOS, Ruby also provides `/usr/bin/rake`. Check that `which rake` shows the CLI
you just installed, normally `~/.cargo/bin/rake`. If it shows Ruby's, put Cargo first:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
rehash # zsh; use hash -r in bash
which rake
```

See [development and reload](development.md) for build options, and
[Chrome DevTools](devtools.md) for connecting the inspector.

## Run in the browser

From the generated app:

```sh
rake dev --target web
```

Use a WebGPU-capable browser. The `wasm32-unknown-unknown` target is installed by
the generated toolchain file. Install a `wasm-bindgen-cli` matching the resolved
version in `Cargo.lock`; the builder prints the exact command if yours is
missing. The template includes the shared library entry point and `web/index.html`.
Rake draws to a GPUI canvas rather than HTML. Kit remains native-only.
See [browser apps](web-applications.md).

To explore a larger app from this repository, run `rake dev --package rake-notes`
or `rake dev --target web --package rake-notes`. The [Foundation](../examples/foundation/README.md),
[Desktop](../examples/desktop/README.md) and [Fieldnotes](notes.md) examples show
loading, guards, shared services and platform features.

## Connect an agent

Agents can discover your app's commands and schemas, run them and read the results. `rake dev`
prints a session directory. In another terminal, point `RAKE_CONTROL_DIR` at its `control`
subdirectory, using your own session name:

```sh
export RAKE_CONTROL_DIR=/tmp/rake-dev-EXAMPLE/control
rake instances --json
rake ctl commands list --json
rake ctl commands run app.greet --args '{"name":"Rake"}' --json
```

Without this variable, the CLI searches its default directory and can return `[]` even while
your app is running.

For more, see [runtime control](runtime-control.md) and the
[CDP agent client](cdp-agents.md), which handles UI input, waits and screenshots.

## Next steps

- [Application shapes](application.md) explains how an app fits together.
- [Routing](routing.md) adds pages, tabs and guards.
- [View syntax](view.md) is a shorter way to write GPUI layouts.
- [The documentation index](README.md) lists every guide.
