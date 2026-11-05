Vendored crates.io ksni 0.3.7 (Unlicense), unmodified dependency/features.

Rake patches:
- Add Handle::update_checked while retaining update's upstream signature. Propagate
  property/menu export errors through the native acknowledgement instead of dropping
  them. A failed signal export does not roll back the local model.
- Observe a closed session-bus event stream, notify watcher_offline and end the worker.
  Upstream's Some(event) select branch otherwise disables itself silently on closure.

A watcher/panel restart on the same bus is recovered by upstream re-registration.
A whole session-bus replacement requires removing/recreating items; no polling
reconnection loop is claimed. Native desktop qualification remains required.
