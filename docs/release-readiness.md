# Alpha support contract

Rake v0.1.0-alpha.1 is a source alpha. Use the pinned release tag and the committed
lockfile; there is no crates.io release or promise of compatibility between alpha
revisions. The Rust minimum is 1.95.0. GPUI CE is pinned in Cargo.toml and Cargo.lock.
Existing public APIs are preserved where practical; breaking changes need migration
notes and updated consumers in the same change. A beta will identify a stable subset
and use deprecation before removing that subset's public APIs.

## Supported scope

| Surface | Alpha scope | Boundary |
| --- | --- | --- |
| macOS | Native source applications | Requires Apple's development tools; signing, notarization and installers are not qualified for this release |
| Windows | Unqualified; outside launch scope | Compilation and boundary tests do not establish GUI or installer support |
| Linux | Unqualified; outside launch scope | Compilation and boundary tests do not establish GUI or installer support |
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
[Notes reference application](notes.md) as executable starting points. The
[Text Toolkit utility](../examples/files/README.md) exercises common platform
services and explicit native/browser processing without routes.

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

## Alpha verification evidence

The launch scope is macOS and browser source applications. Release evidence must
identify the exact commit and distinguish compilation, automated host tests and
real GUI observations. See the release's accompanying evidence for final-commit
readiness results; historical passes below are not final-commit verification.

The Fieldnotes module-loading fix was verified at
`af0cd47b55a4a8b2952dc7d5ae712bc34e130cc0`. All five local readiness groups,
the temporary CLI install and Web development/control flow passed at that revision.
The [complete browser checklist](notes-browser-testing.md#alpha-preparation-rerun-2026-11-03)
records actual Chrome 156.0.8078.80 Guest, Safari 26.6.2 and Firefox 159.0 runs on
macOS 26.6.2, including exact stored text, hidden autosave, stale writers,
interrupted saves, write failure/retry and malformed-checkpoint recovery.
Safari's initial hidden-writer discovery failure and successful retry are recorded.

Real Windows and Linux GUI runs, signing, notarization and installers were not
verified for this source alpha. Browser automation does not establish every IME,
accessibility, mobile input or execution during browser suspension. Hosted
compilation/boundary results establish only the checks actually run. Each release
must retain its own final-commit evidence and the limits of any reused manual run.
