# Installed Rust profiles

Selected: rust-library, rust-cli, rust-async, rust-native-wasm, rust-workspace.

Use [the baseline](AGENTS.md), [architecture](architecture.md),
[testing](testing.md), [documentation](rustdoc.md), and [config guidance](configs/README.md).

## Apply a profile

1. Read manifests, toolchain configuration, Cargo aliases, scripts, and CI. Record
   the supported edition, MSRV, features, targets, and required native libraries.
2. Select only profiles that match the code being worked on. Profiles add guidance;
   they do not choose dependencies, change the toolchain, or scaffold an application.
3. Establish actual commands for format, check, lint, test, docs, and any target
   build. Keep fixes separate from checks and generator steps explicit.
4. Follow existing project choices. When adding a dependency, inspect its installed
   version's docs, enabled features, platform support, and ownership requirements.

For mixed TypeScript/Rust projects, combine a TypeScript profile with the Rust
profiles. Apply each language's baseline to its own code. Verify both consumers
when a generated contract or binding changes.

## Library

- Keep the public surface deliberate, including error and option types. Use an
  external-consumer test and doctests to verify imports and real usage.
- Document ownership, failure, units, and important costs. Concrete typed errors
  help callers recover; keep application logging/reporting outside reusable logic.
- Treat features and the MSRV as part of the supported contract. Test documented
  feature combinations, including no-default-features if that is supported.
  `--all-features` is useful only when the enabled combination is valid.
- Before a requested release, inspect the actual package and public compatibility
  using the project's release process. Routine development does not imply publishing.
- **Verify:** focused behavior tests, consumer tests, doctests, docs, and supported
  configurations. A single default-feature check covers only that configuration.

## CLI

- Keep `main.rs` focused on arguments, runtime setup, output, and exit status.
  Extract reusable/testable behavior into the library when there is substance
  to separate; avoid a library made solely of forwarding wrappers.
- Preserve stdout for the advertised machine-readable or user result, stderr for
  diagnostics, and deliberate exit codes. Validate paths and inputs at the boundary.
- Keep configuration precedence explicit. Inject operation-specific configuration
  instead of reading environment variables throughout domain code.
- Use an existing argument parser when present; a new dependency is justified by
  actual command complexity. Handle interruption and child-process cleanup.
- **Verify:** argument errors, success/failure exit codes, stdout/stderr, file
  outputs, and relevant cleanup through a spawned binary. In Cargo integration
  tests, `env!("CARGO_BIN_EXE_<binary-name>")` identifies a built binary; replace
  the name with the actual Cargo binary target.

## Async service

- Preserve the selected runtime. Keep pure domain rules independent of runtime
  handles; construct clients, pools, and task owners at the application boundary.
- Define cancellation and shutdown for spawned tasks. Observe task failures and
  join completion where required; dropping a handle is not necessarily cancellation.
- Bound queues and concurrency when producers can outpace consumers. Avoid holding
  a synchronous lock guard across `.await`; choose the lock and scope by the actual
  synchronization requirement. Do not default every shared value to `Arc<Mutex<_>>`.
- Move blocking CPU/native/file operations to the runtime's appropriate boundary.
  A blocking worker may continue after its async handle is cancelled; give long
  operations their own stop mechanism when needed.
- **Verify:** success/failure, timeouts, cancellation, and shutdown without arbitrary
  sleeps. Use the runtime's existing test/time facilities and real-boundary suites
  for claims about an external service.

## Native and WASM

- Keep portable representations and algorithms in a core module/crate. Place JS,
  windowing, GPU, and native-library integration in explicit adapters.
- Preserve the chosen WASM target and host: browser, worker, or WASI. They have
  different APIs. Do not assume native filesystem, thread, clock, or blocking
  behavior exists in a browser build.
- Keep target-specific dependencies and feature gates aligned. Do not apply a
  native async runtime configuration to browser code without checking support.
- Treat numeric conversion, buffer ownership, resource disposal, and error
  translation as public binding contracts. Keep generated schemas and bindings
  attached to their source and regeneration command.
- **Verify:** portable tests, native checks, an explicit WASM target check, and
  the relevant host test/build. `cargo check --target wasm32-unknown-unknown`
  establishes compilation for that target; it does not exercise a browser or GPU.
  Use the project's installed browser/WASM test harness for changed host behavior.

## Workspace layer

- Add crates for real dependency, consumer, platform, or build boundaries. Use
  modules for ordinary organization inside a crate. Keep the dependency graph
  directed and explain each crate's owner and public surface.
- Share versions through `[workspace.dependencies]` when useful; each consumer
  opts in. Keep feature/default-feature choices explicit and inspect their effective
  resolution when a target unexpectedly pulls in dependencies.
- Put shared lints under `[workspace.lints]` and opt each member in with
  `[lints] workspace = true`. Workspace declarations alone do not activate them.
- Preserve the workspace resolver, edition, lockfile, and `default-members`.
  Root commands without explicit selection may not cover every member.
- **Verify:** use `-p` for focused iteration and relevant dependents for interface
  changes. Run the intended workspace/target matrix before claiming workspace
  compatibility; native tools and GPU prerequisites remain explicit.

## Common skills and optional dependencies

Every Rust profile includes [Rust development](../../skills/rust-development/SKILL.md)
for ownership diagnostics, API changes, and Rust-specific implementation/review.
The skill supports this workflow's project conventions and remains usable on its own.

Prefer standard Cargo tooling initially. nextest, property tests, snapshots,
benchmarks, dependency audits, and coverage are additions for demonstrated needs.
Keep installed runners and relevant existing dependencies. A profile does not
mandate Tokio, an error derive crate, a web framework, or an optimization library.
