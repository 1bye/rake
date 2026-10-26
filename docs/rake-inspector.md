# Rake Inspector application

`examples/inspector` is an Elements-only Rake application. Its shared native and
browser UI is authored with the experimental `rake::view!` macro. It has its own
runtime, typed connection service, routed entry and managed observation activity.
It observes the target through the existing protocol-1 runtime control API.

From the Rake workspace, build the CLI and attach to a development session:

```sh
cargo build -p rake-cli
# A native application bundle, with a private loopback broker owned by this command:
target/debug/rake inspect --elements --target desktop --session /tmp/rake-dev-EXAMPLE
# The same view compiled to Wasm, hosted on a dedicated inspector origin:
target/debug/rake inspect --elements --target web --session /tmp/rake-dev-EXAMPLE
```

Both accept `--instance UUID|APPLICATION_ID`, `--port`, `--no-open`, and `--json`.
Without a session, discovery uses the normal private control directory. An
ambiguous application ID requires choosing an exact runtime in the target menu.
`--no-open` builds and serves without opening a window or browser. Terminating the
command closes its broker and removes temporary artifacts. Closing the native
Inspector window exits that Inspector and its broker; the target keeps running.

This source preview requires the Rake workspace, Python 3.11+, the native GPUI
build prerequisites, and the `wasm32-unknown-unknown` target for Web. Web also
requires the `wasm-bindgen-cli` version recorded in `Cargo.lock`, as with other
Rake Web builds. The CLI reports the required install command if it is missing.
The existing multi-panel HTML DevTools remains available through `rake inspect`.

## Elements

The frontend provides a target picker, semantic/type/source filter, Pause/Resume,
explicit Refresh, disconnect/reconnect, resizable panes, and dark/light themes.
Styles, Layout, and Source show owned metadata from the target. The divider also
supports Left/Right and Home/End when focused. At narrow widths, the panes stack.
The filter supports platform text input, IME, selection and clipboard shortcuts.

Capture IDs belong to one exact runtime and capture. A replaced capture leaves
selected details frozen and visibly marked; another node with the same semantic
ID is never silently substituted. Late replies from an old selection or target
are ignored. Selection immediately preserves the exact tree metadata, so a
target redraw before the confirming lookup cannot erase the displayed details.
Only an advertised successor with matching application identity and
`restored_from` can replace a disconnected target automatically.

Observation runs in an entry-owned Rake activity and uses cancellable HTTP with
bounded responses. The application sends only Elements and runtime-info queries.
The dedicated development broker supports the same authenticated native/Web
transports as the existing DevTools. It serves the frontend at
`/__rake/elements/`; ordinary application origins cannot gain native IPC access by
serving this frontend. Credentials stay out of public assets, query parameters,
and displayed connection errors.

## Scope and verification

This first application implements Elements only. It shares the existing
[inspection coverage](element-inspection.md): explicit Rake boundaries, authored
base styles, measured bounds before clipping, source locations and semantic IDs.
It does not enumerate opaque GPUI descendants or provide computed CSS, style
editing, a pixel picker, or the other DevTools panels. Trees are bounded to the
runtime capture limit of 256 boundaries. Filter editing is single-line and bounded
to 1 KiB. The frontend inherits the experimental status of `view!`.

```sh
cargo test -p rake-inspector --no-default-features --locked
cargo clippy -p rake-inspector --all-targets --locked -- -D warnings
cargo clippy -p rake-inspector --lib --target wasm32-unknown-unknown --locked -- -D warnings
python3 scripts/test-inspector.py
```

The Rust tests cover exact shared query envelopes over real loopback HTTP, launch
validation, response bounds, stale captures, selection races and reload identity.
The Python suite covers the real broker's asset isolation, authentication and
native/Web forwarding. GPU/native interaction and browser rendering require a
real host; compilation alone does not establish them.
