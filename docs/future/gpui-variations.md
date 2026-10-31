# GPUI variations: future integration handoff

Status: deferred. Rake currently supports the pinned GPUI Community Edition
revision in `crates/rake/Cargo.toml` and `crates/rake-ui/Cargo.toml`. This document
describes a future direction, not an implemented compatibility guarantee.

## Goal

Allow an application to choose a supported GPUI version or fork at compile time:
Community Edition, upstream GPUI, another performance-oriented fork, or a custom
Rake-oriented fork. Each application and its widget libraries must agree on the
same GPUI dependency. Types such as `Entity`, `Window`, and `Element` from distinct
crate versions/sources are not interchangeable. Runtime backend switching and
mixing their UI objects in one tree are outside the initial scope.

## Integration approach

- Preserve GPUI-independent protocol, routing/data models, diagnostics, and client
  code. Isolate rendering, window lifecycle, input, task scheduling, HTTP/platform
  integration, accessibility, and element inspection where they touch GPUI.
- For an API-compatible fork with compatible package names and versions, first
  try an application-workspace Cargo `[patch]` override. Include platform/helper
  crates as necessary; a patch does not translate changed APIs.
- For diverging APIs, add dedicated integration crates depending on the shared
  Rake domain contracts. Extract the actual differences when implementing a second
  backend; do not introduce a generic wrapper for every GPUI method in advance.
- Features can enable optional integration dependencies, but Cargo feature
  unification is additive. Prefer separate integration crates if mutually
  exclusive backend features would conflict in downstream dependency graphs.
- Keep a clear import path for the selected backend. Existing `rake::gpui`
  reexports and macro-generated paths help, but Rake, `rake-ui`, macros, and
  application widgets must all resolve compatible concrete types.

## Capability differences

Advertise optional capabilities instead of restricting every backend to the least
capable implementation. A custom fork may expose full element traversal, stable
element lifetimes, exact box geometry, style overrides, or rendering/profiling
hooks. Other backends can retain boundary-based inspection with explicit limits.
Native and Web support must be advertised and tested separately.

Keep inspection snapshots and the control protocol renderer-independent. The
Rake inspector and CDP adapter should consume shared owned metadata; they should
not import each fork's UI types or reconstruct unsupported observations.

## First implementation sequence

1. Choose a concrete second fork/revision and compare the APIs Rake actually uses.
2. Try a compatible dependency override before designing an adapter abstraction.
3. Extract only the integration boundaries required by that second implementation.
4. Migrate UI macros, examples, platform helpers, and widget consumers together.
5. Publish a supported revision/target/capability matrix and test each combination
   in CI. Include consumer builds, window/input behavior, inspection/picking,
   cleanup, and an explicit Web build/runtime check where supported.

Do not promise arbitrary GPUI revisions. Each supported version/fork needs an
owner, compatibility tests, and an upgrade policy. Native compile checks alone
do not establish Web, GPU, or interaction compatibility.

References: [Cargo dependency overrides](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html)
and [Cargo features](https://doc.rust-lang.org/cargo/reference/features.html).
