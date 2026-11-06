# Unopened ksni proposal

This is proposal text only. No upstream issue or pull request was opened.
Rake uses unmodified crates.io ksni 0.3.7 through its public API.

Proposed subject: Expose update export errors and observe session-bus stream closure

`Handle::update` returns the local callback result after the service processes an
update message, while `service.update(&conn)` signal-export errors are discarded.
An application therefore cannot distinguish a locally changed model from a failed
D-Bus export. Preserve the existing `update` signature, and optionally add a
checked API that returns the export result through the update acknowledgement.

The service select loop matches `Some(event)` from the watcher-owner signal stream.
When the stream closes, it can disable that branch without notifying the owner.
Handle `None` explicitly, call watcher_offline with a connection diagnostic, close
the connection and stop the worker. Neither this change nor a checked update API
would establish reconnection to a replacement session bus.

Suggested verification: isolated D-Bus export/update signal checks, connection loss
while an update is pending, stopped-service admission, worker cleanup, and unchanged
behavior/signature for existing Handle::update consumers on both runtime features.

Current Rake limitation: is_closed/None from public Handle::update and watcher
callbacks are observable; per-signal export failures and reliable detection of
whole-bus disconnection are not. Receipts acknowledge the local model, and the
application must recreate registrations after a whole session-bus replacement.
