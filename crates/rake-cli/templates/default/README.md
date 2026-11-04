# {{name}}

A Rake app with two retained pages, a persistent layout, a theme and an agent command.

```sh
rake dev
rake dev --target web
```

Requires Rust 1.98.0, native development tools, and Python 3.11+. For Web, use a
WebGPU browser and a `wasm-bindgen-cli` matching the version in `Cargo.lock`.
The Web builder prints the installation command if it is missing. The first
`rake dev` or `rake build` resolves dependencies and creates `Cargo.lock`; commit
that file. Later builds use it with `--locked`.

Edit `src/lib.rs` while `rake dev` runs to rebuild and restart. Home's counter is
retained in route history: increment it, open About, then use Back. The layout's
navigation count survives page changes.

In another terminal, use the session directory printed by `rake dev`:

```sh
export RAKE_CONTROL_DIR=/tmp/rake-dev-EXAMPLE/control
rake ctl commands list --json
rake ctl commands run app.greet --args '{"name":"Rake"}' --json
```

Change the application ID (`{{application_id}}`) in `src/lib.rs` and
`package.metadata.rake.bundle.identifier` in `Cargo.toml` together. Rake is an
alpha; APIs can change between revisions. The dependency is pinned to the CLI's
build revision (or the explicit local path/revision passed to `rake new`).

Keep the `pest_stack_guard` patch in `Cargo.toml`: it carries Rake's parser stack
guard fix for native-to-Wasm builds. Root manifest patches are not inherited from
a dependency. Update its revision together with Rake's.
