# Standalone loading and updates

Production Web bundles also show deployment version checks and **Reload to update**.
Reload reuses quit guards and finite-work policy. Offline packaging is enabled in
Cargo metadata; unbundled `build-web.py` output keeps development behavior.
See [browser applications](../../docs/web-applications.md) for deployment order,
retaining old build assets, worker execution and offline lifecycle.

The **Copy message** button uses `runtime.services().platform()`. It initiates the
clipboard request inside the click handler, retains the observing task in the view,
and displays permission failures. Browser availability is detected at runtime.
See [platform services](../../docs/platform-services.md) for file and preference APIs.

This route-free `rake::view!` app constructs two view-bound handles:

```rust
message: runtime.load(cx, load_message),
updates: runtime.bind_updates(cx),
```

Rake starts the loader, retains typed errors, rejects stale retry results, and
redraws the view. Releasing the view cancels its loading and update observation.
The continuous update observer does not enter the finite-work quit wait. The
separate managed-job button demonstrates finite work that normal quit waits for.

From the repository root, serve the sample JSON in one terminal:

```sh
python3 -m http.server 8080 --bind 127.0.0.1 --directory examples/foundation/web
```

Run the native app in another:

```sh
cargo run -p rake-foundation
```

Stop the server and choose Reload / Retry to see the loader's error state. Restart
the server and retry to recover. An installed build supplies update policy; an
unbundled run shows unavailable update controls. The [desktop example](../desktop)
contains the complete packaging/update configuration.

For the browser build, use `python3 scripts/build-web.py --example foundation`
and `python3 scripts/serve-web.py --example foundation`. Native update installation
is unavailable in the browser.
