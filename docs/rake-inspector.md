# Rake Inspector application

Legacy prototype, retained as historical implementation documentation. Stock
[Chrome DevTools](devtools.md) is the supported frontend; this application is not
part of the supported DevTools workflow.

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
The interface starts in a light theme, with a compact toolbar and a 55/45 split.
Connection, capture and coverage details live in the bottom status bar. The theme
button switches both panes and their inputs together. Its typed, app-owned
`Palette` derives `rake::Theme` and is declared once with `rake::theme!`.
Setup and switching use `rake::theme::install`; named classes and the shared
focus preset read that ambient value. Static styling in `view.rs` uses the
[class grammar](experimental-view.md#class-grammar), reducing style closures
from 84 to 10. Computed layout, captured swatches and tree semantics keep GPUI
builders. The explicit-palette property helper also retains its builders so its
construction-only identity test needs no ambient application setup.

The tree uses monospace type names, syntax-colored semantic `id` attributes,
indentation guides, real disclosure controls, closing tags and ancestor
breadcrumbs. Find opens from the toolbar or Cmd/Ctrl+F and includes matching
ancestors. A new query expands its ancestor path. Disclosure state belongs to
one capture and resets when it is replaced; stable identities across captures
are a separate runtime capability. The filter supports platform text input,
IME, selection and clipboard shortcuts.

The tree is one keyboard tab stop. Up/Down select visible nodes, skipping closing
tag annotations. Left collapses an expanded branch or selects its parent; Right
expands a collapsed branch or selects its first child. Home/End select the first
or last visible node. Page Up/Down move by the visible tree height, and Enter or
Space toggle a branch. Text inputs and the pane divider retain their own keys.
The selected tree item exposes its level, expansion and selection state to native
accessibility, using the tree's active-descendant focus.

Reveal selection in the toolbar expands the current ancestors, clears a tree
filter only if it hides that node, and scrolls the selection into view. Breadcrumbs
also reveal their exact current node. A replaced capture's frozen selection cannot
be revealed by substituting a node with the same semantic ID. Collapsing an ancestor
of the current selection explicitly selects that ancestor instead.

The tree keeps its horizontal and vertical scroll offsets through polling and
capture updates. Keyboard selection changes only the vertical offset needed to
show the row. Styles, Layout and Source each retain a separate scroll position
when switching panes. Positions clamp when content shrinks; explicitly choosing a
runtime resets the pane positions. Active selection, inactive selection, pointer
hover and keyboard-focused controls have distinct treatments in both themes.

Styles displays the captured authored refinements as declarations grouped by
layout, spacing, appearance and typography. Supplied dimensions and edges expand
into readable property rows, preserving their units. Recognized solid colors get
swatches; unknown structured values retain an expandable raw JSON view. The
property filter matches names and supplied values. Its source link switches to
the captured boundary's Source details; it does not claim per-declaration source
provenance inside arbitrary closures. Automatic `view!` nodes show their authored
tag and style attribute locations/expressions in Source, including inline paragraph
styles attached to the paragraph. Layout measurements remain in the Layout pane.
These blocks show GPUI
base refinements. Computed shows live resolved values with per-property observed
origins (defaults, inherited text context, authored base, interaction changes, measured
layout). Equal-valued interaction rules and opaque per-method Rust locations remain
unavailable. Layout lists separate margin, border, padding and content bounds, plus
the visible intersection. Unknown auto/percentage edges and scrollbar metrics are
marked unavailable, rather than displayed as zero or duplicated border bounds.

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
editing, an uninstrumented pixel picker, or the other DevTools panels.
Its Pick control and tree hover use the runtime’s leased instrumented-boundary
inspection controls, including clipping and automatic input restoration. Trees are bounded to the
runtime capture limit of 256 boundaries. Filter editing is single-line and bounded
to 1 KiB. The frontend inherits the experimental status of `view!`.

Selected current boundaries offer **Scroll into view**, **Focus element**, and
**Open Rust source**. Navigation uses registered application handles; unavailable
actions and replaced captures are disabled. Source opens through the authenticated
host using the system default editor and the workspace restriction described in
[Elements navigation](element-inspection.md#elements-navigation).

```sh
cargo test -p rake-inspector --no-default-features --locked
cargo clippy -p rake-inspector --all-targets --locked -- -D warnings
cargo clippy -p rake-inspector --lib --target wasm32-unknown-unknown --locked -- -D warnings
python3 scripts/test-inspector.py
```

The Rust tests cover exact shared query envelopes over real loopback HTTP, launch
validation, response bounds, stale captures, selection races and reload identity.
Presentation tests cover keyboard traversal of filtered and collapsed trees,
closing-tag annotations, page movement and minimal reveal offsets.
The Python suite covers the real broker's asset isolation, authentication and
native/Web forwarding. GPU/native interaction and browser rendering require a
real host; compilation alone does not establish them.
