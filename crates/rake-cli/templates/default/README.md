# {{name}}

A Rake app with two retained pages, a persistent layout, a theme and an agent command.

```sh
rake dev
rake dev --target web
```

Requires Rust 1.98.0 and native development tools. For Web, use a
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
release version, checkout revision, or the local path or revision passed to
`rake new`.

After edits, run `rake format` then `rake lint`. Agent instructions live in
`AGENTS.md`; `CLAUDE.md` imports that file. Refresh its generated block with
`rake lint --guide=agents --update AGENTS.md`.

Optional PostToolUse feedback can run `rake lint --hook`. It reads the edit
payload from stdin and returns compact findings without blocking the edit.

On Windows, use `rake build` and `rake bundle`; development supervision and
native local inspection currently require Unix.

On Apple Silicon macOS, `rake dev --live` installs the pinned experimental Blaze
bundle and applies supported edits without restarting. Layout changes restart
the app; only registered checkpoints are restored. Use `--require-state` to block
restarts when a checkpoint is missing. Plain `rake dev` uses Cargo.

The generated `live` and `devtools` features let Cargo lock the optional runtime
and inspection dependencies. The `gpui-fast` alias keeps the alternative GPUI
backend's version constraints in the lockfile; it is not enabled by `--live`.
