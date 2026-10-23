# Ownership decisions

## Borrow-checker diagnostics

Identify the owner and the two operations whose lifetimes overlap. Read the first
diagnostic and its spans before adding annotations. Useful fixes include ending a
borrow after extracting needed data, operating on disjoint fields, changing an API
to accept only the resource it uses, or transferring an owned value at the boundary.

```rust
let item = items.get(index)?;
let id = item.id; // Copy the small identifier needed by the mutation.

items.remove(index);

Some(id)
```

This example assumes `id` is `Copy` and `items` is a vector; copying the identifier
lets the borrow end before removal. If the caller needs the entire removed value,
returning the owned result of removal may be the clearer operation instead.

Do not return references to temporary/local values. Own the return value or borrow
from an existing owner with a justified relationship. An explicit lifetime describes
a relationship; it cannot make a local allocation live longer.

## Choosing sharing

- `&T`: temporary read access; `&mut T`: exclusive temporary access.
- Owned `T`: independent lifetime or ownership transfer.
- `Rc<T>`: shared ownership within a compatible single-threaded context.
- `Arc<T>`: shared ownership across threads when the contained type supports it.
  `Arc` alone does not provide shared mutation or make arbitrary `T` thread-safe.
- `RefCell`, locks, or atomics: a specific interior-mutation/synchronization need,
  with explicit borrowing/contention/ordering behavior.

Prefer owned fields in long-lived application structs when borrowed fields would
spread lifetimes through otherwise simple APIs. Borrow locally for operations.
Use a deliberate clone when independent state is required; avoid lifetime complexity
that costs more than copying a small value. Measure expensive data copies.

## Tasks and resources

Check whether the runtime's spawn operation needs `Send` and/or `'static`. Move
owned captures into tasks or use an existing scoped mechanism when appropriate.
Do not add `'static` bounds, leak allocations, or add unsafe `Send`/`Sync` impls to
conceal missing ownership. Explain who joins/cancels the task and handles its errors.

Shorten lock scopes before awaiting I/O. Moving synchronous work to a blocking worker
does not make it cancellable; preserve resource cleanup and bounded concurrency.
Treat channels as ownership transfer with a capacity and shutdown contract.

## Derived data

An owned snapshot can outlive the source and becomes stale when the source changes.
A borrowed view tracks a source lifetime. A cache adds reuse policy and identity.
State which one an API returns. Confirm whether clones share underlying allocations,
copy them, or detach only on mutation, and test that contract when callers rely on it.
