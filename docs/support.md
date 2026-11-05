# What Rake supports

Rake is an alpha. Pin the version and commit your lockfile. APIs can change between alpha releases. The Rust minimum is 1.98.0. `gpui-pre` 0.3.11 is pinned in Cargo.toml and Cargo.lock.
Existing public APIs are preserved where practical, and breaking changes come with migration
notes.

## Supported scope

| Surface | Alpha scope | Boundary |
| --- | --- | --- |
| macOS | Native source applications | Requires Apple's development tools; signing, notarization and installers are not qualified for this release |
| Windows | Unqualified | Compilation and boundary tests do not establish GUI or installer support |
| Linux | Unqualified | Compilation and boundary tests do not establish GUI or installer support |
| Browser | Single-host canvas applications on WebGPU-capable browsers | HTTPS or localhost, GPU availability, browser storage and permission policies |
| Headless | Portable services, commands, tasks and routing state | Caller supplies the executor; no invented window or browser authority |

Support doesn't mean every API exists everywhere:

- Native secondary windows, helper processes and native installer updates are unavailable in a browser.
- Browser background work can be suspended.
- `view!` is the standard UI syntax, included with the default `gpui` feature. It isn't a complete widget toolkit or a verified browser screen-reader interface. APIs follow the alpha policy above.
- Web rendering uses GPUI's canvas and GPU, not HTML layout.

Check capabilities and handle errors at the operation boundary, rather than branching only on the compilation target.

Beta will identify a stable API subset and deprecate before removing any of it. It also needs maintained real-GUI coverage on supported
platforms, keyboard, IME, clipboard and accessibility checks, display-scale and window lifecycle
coverage, recovery and upgrade journeys, and sustained workload measurements.

[Getting started](getting-started.md) covers installation.
