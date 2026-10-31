# Alpha support contract

Rake is an unpublished alpha candidate. Use a pinned Git revision and the committed
lockfile; there is no crates.io release or promise of compatibility between alpha
revisions. The Rust minimum is 1.95.0. GPUI CE is pinned in Cargo.toml and Cargo.lock.
Existing public APIs are preserved where practical; breaking changes need migration
notes and updated consumers in the same change. A beta will identify a stable subset
and use deprecation before removing that subset's public APIs.

## Supported scope

| Surface | Alpha scope | Boundary |
| --- | --- | --- |
| macOS | Native applications, distribution, local signing and updates | Requires Apple's development tools; notarization requires a configured profile |
| Windows | Native applications, installers, signing, Velopack | MSVC/Windows SDK on the build machine; native development control IPC is not implemented |
| Linux | GPUI X11/Wayland applications and supported distribution formats | Development libraries on the build machine; package-manager and sandbox behavior differ |
| Browser | Single-host canvas applications on WebGPU-capable browsers | HTTPS or localhost, GPU availability, browser storage and permission policies |
| Headless | Portable services, commands, tasks and routing state | Caller supplies the executor; no invented window or browser authority |

Platform support does not mean every API exists everywhere. Native secondary windows,
helper processes and native installer updates are unavailable in a browser. Browser
background work can be suspended. The `experimental-view` feature is opt-in; it is not
a complete widget toolkit or a verified browser screen-reader interface. Web rendering
uses GPUI's canvas/GPU, not HTML layout. Check capabilities and handle errors at the
operation boundary rather than branching only on the compilation target.

## Reproducible source installation

From a clean checkout at the revision being evaluated:

```sh
rustup show
cargo install --path crates/rake-cli --locked
cargo run -p rake-foundation --locked
```

The repository's rust-toolchain.toml installs the selected compiler and Web target.
Native build libraries are development/build prerequisites, not programs end users
must install beside a correctly packaged app. Distribution-specific runtime libraries
and sandbox permissions still belong in the package configuration.

For Web, install wasm-bindgen-cli at the exact wasm-bindgen version in Cargo.lock
(scripts/build-web.py reports the required command), then build and serve:

```sh
python3 scripts/build-web.py --example foundation
python3 scripts/serve-web.py --example foundation --port 8080
```

Use the [standalone example](../examples/foundation/README.md),
[desktop example](../examples/desktop/README.md), and
[Notes reference application](notes.md) as executable starting points.

## Release gates and evidence

Run `python3 scripts/check-readiness.py --list` to see the bounded check groups.
The `browser` group uses a real isolated Chrome instance and a Rust Wasm fixture;
`web` includes compilation and deterministic adapter/cache tests. See
[qualification](qualification.md) for their exact boundaries and prerequisites.
Each invocation writes an evidence report with the revision, dirty-worktree state,
commands, durations and exit codes. A failed prerequisite fails the invocation;
compilation is never recorded as a browser or native GUI test.

Before tagging an alpha candidate:

1. Run portable, native, Web and UI groups locally on the release revision.
2. Run the focused Windows/Linux `app-readiness` matrix once for the final revision.
   The broader signing/installer workflows are needed when those paths change.
3. Perform real GUI checks from [browser testing](notes-browser-testing.md) and the
   native startup/layout smoke scripts; attach results with their OS/browser versions.
4. Review API/docs changes, migration notes, known limitations and package contents.
5. Tag the verified clean revision as `v0.1.0-alpha.N`; publish source/checksums and
   the evidence. Tagging does not turn unexecuted manual gates into passes.

Beta additionally requires maintained real-GUI coverage for the supported platforms,
keyboard/IME/clipboard and accessibility checks, display-scale/window lifecycle
coverage, recovery/upgrade journeys and sustained workload measurements. Synthetic
CPU benchmarks and compile checks do not establish these properties.

## Implementation phases

1. Define the alpha contract, reproducible setup and evidence-producing checks.
2. Expose common files, clipboard, preferences and URL services with explicit errors.
3. Add browser workers, offline deployment assets and guarded deployment reload.
4. Add regression/release coverage and document remaining real-host qualification.
5. Exercise the APIs in a file-oriented reference application and update examples.

This sequence implements release infrastructure and platform behavior. A release is
qualified only by checks actually executed against its revision, not by this checklist.
