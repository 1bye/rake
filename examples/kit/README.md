# Optional GPUI Kit example

Run on macOS with `cargo run -p rake-kit-example --features kit,devtools`.
Open a components tab, increment its Kit button, open its Kit dialog, switch the
shared theme from either page or dialog, and switch tabs to check retained state.
`RAKE_DEV_CONTROL=1` enables the local inspector control endpoint.

Kit is excluded from the example's default features, so ordinary workspace
checks do not turn it on. Kit 0.9.0 is explicitly native-only in Rake: its Web
platform defaults enable multithreading and require nightly. Core Rake and
Base's experimental views remain available on stable single-threaded Web.

The table and buttons have explicit inspection boundaries. Geometry, authored
wrapper styles, source sites, declared handlers and explicitly supplied focus
handles are visible. Kit's private descendants, internal resolved styles and
unregistered control state cannot be enumerated or edited through Rake.
Rake-authored dialog content is fully inspectable under the window-wide capture.
