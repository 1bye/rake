# Refactoring Rust boundaries

Map the affected public exports, actual consumers, state fields, helper callers,
tests, and enabled feature/target branches. Run the existing focused baseline.
Use that evidence to choose the smallest cohesive ownership change.

## Moving behavior

Group by domain and then named responsibility. Several `impl` blocks for a local
type can live in separate modules. That improves navigation, but a state object
whose fields are mutated everywhere may need actual subsystem owners as well.

Move a helper to its real consumer. Introduce a shared module only when several
consumers need the same contract. Prefer precise names over `common`, `manager`,
or `utils` when a domain name describes the relationship.

Keep state and operations together when they enforce an invariant. Split derived
frame/request data from persistent resources when their lifetimes differ. A thin
orchestrator should reveal execution order, dependencies, and failure propagation.

## Preserving the consumer

Keep public import paths through deliberate reexports when required. Verify that
public result, option, and error types are nameable from outside the crate. Preserve
feature gates on moved items; check both the relevant enabled and disabled builds.

Use internal tests for private behavior and consumer tests for public usability.
Do not widen visibility to relocate a test. A tests module is a descendant of its
parent; an unrelated sibling suite does not inherit all of another module's privacy.

Structural moves should preserve serialization, numeric precision, iteration order,
ownership/clone costs, and cleanup behavior. Treat a changed invariant, algorithm,
or data representation as a separate design decision even if it is in the same task.

## Verification proportional to the change

- Module move: existing behavior tests, compilation, and public consumer imports.
- Public API/error change: affected callers, consumer tests, and doctests.
- Feature/platform boundary: affected supported configurations and host checks.
- Resource/concurrency change: cleanup, cancellation, ordering, and contention where relevant.
- Hot-path algorithm/storage change: correctness cases and the existing benchmark.

Update module docs after the structure settles. Report the actual resulting owner
and evidence; do not use smaller files or fewer lines as the only success criterion.
