# Browser application behavior

The browser host uses a single-threaded GPUI canvas. Ordinary asynchronous loaders
run on its UI loop. They do not move CPU work into another thread. Browser lifecycle,
storage permissions, font assets and WebGPU support remain explicit host boundaries.

## CPU work

`rake::web::run_worker(module, &json, timeout)` starts one same-origin module worker. It receives one JSON string and replies with one, both capped at 1 MiB.

The returned future owns the worker, so completion, error, timeout and dropping it all terminate the worker. Move it into a managed task or loader to inherit cancellation. No Rust closure or GPUI entity crosses this boundary. For example, a worker can use:

```js
self.onmessage = event => {
  const {text} = JSON.parse(event.data);
  self.postMessage(JSON.stringify({characters: [...text].length}));
};
```

Workers need a permitted same-origin script URL and CSP `worker-src`; they do not
require SharedArrayBuffer or cross-origin isolation. Supply an appropriate Wasm/JS
worker for heavier workloads. Native CPU tasks continue using the background
executor. Browser suspension can still delay timers and task completion.

## Offline deployment and updates

Production `rake bundle --target web` emits a content identity and a small `rake-version.json` endpoint.

- `Deployment::from_document()` captures the running build.
- `deployment.check().await` checks the same-origin endpoint without caching or redirects, with a deadline and body limit. It returns a changed build even when the human-readable version matches.
- Foreign application or root identities fail.

Unbundled development pages and native hosts have no deployment metadata.

Enable offline assets in Cargo metadata:

```toml
[package.metadata.rake.bundle.web]
base_path = "/tools/"
routing = "path"
offline = true
```

The same application can keep macOS/Windows/Linux signing, installer and update
sections in this manifest. Native sections are consumed by the native Rust pipeline;
browser payloads use the explicit Web directory and common `assets` tree. Declaring
a native resource/helper does not make it executable or accessible in a browser.

Offline bundles place payloads below `rake-builds/<content-id>/`. The generated page base points at that immutable payload, while route URLs keep the configured app root.

The service worker precaches a complete immutable build, and a failed installation deletes the partial cache. Navigation is network-first, falling back to the active build's cached shell. Only immutable build resources and the shell are cached, not API requests, version checks, credentials or arbitrary downloads.

To deploy:

1. Upload **new immutable build directories first**.
2. Atomically replace the root index, version metadata and worker script.
3. Keep previous build directories while old tabs might request them.

Host the version, worker and index endpoints with a revalidation or no-store policy; the generated nginx example sets immutable asset headers. Use HTTPS (localhost is fine for development).

Offline readiness appears on the document as `data-rake-offline=ready`, meaning installation and caching completed. Registration or cache failures don't prevent online startup and remain visible in the console. The first, uncontrolled page gets a service-worker controller on its next navigation, and its existing requests aren't retroactively controlled.

There is no automatic `skipWaiting`, `clients.claim` or forced reload of other tabs.
An old tab keeps its build resources; another tab may load the new network deployment.
An installed worker activates through the browser's normal lifecycle. Offline mode
is a cache, not a guarantee that browser storage will never be evicted.

Call `runtime.reload_web_update(&deployment, QuitOptions::default()).await` from view-owned observing work after the user chooses to reload. It rechecks the server and submits the existing guarded quit, so save, discard, cancel, application guards and finite work settle first. The runtime owns the final handoff, so disposing a routed page can't cancel reload after approval.

- Don't await this inside a finite task that quit must wait for.
- A browser navigation failure after approval requires a manual reload.
- A forced tab close or crash can't await asynchronous guards, so persist drafts before approving.

The [foundation example](../examples/foundation/README.md) shows version checking and
guarded reload alongside native update binding. Browser deployments use origin/TLS
trust, not Sparkle/Velopack signatures. Durable app/document state must be restored
by the application after reload; development checkpoints are not production storage.
