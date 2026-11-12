# OS notifications

Use `services.platform()` for application-owned notifications. This service posts
plain-text OS notifications, separate from in-app toasts. It uses the existing
native host delivery API and the browser Notification API on Web. No feature beyond
Rake's default `gpui` feature is needed to post or dismiss. Headless builds expose
permission queries that return `platform::Error::Unsupported`.

```rust
use rake::platform::{Notification, NotificationOutcome, PermissionState};

// In a click handler: permission requests begin here, before polling the future.
let permission = runtime.services().platform().request_notification_permission();
// Retain the result with runtime.action(cx, permission), then show on a later click.

let request = runtime.services().platform().show_notification(Notification {
    tag: "export-42".into(),
    title: "Export finished".into(),
    body: "Open the exported document".into(),
    activation_url: "myapp://exports/42".into(),
}, cx);
// Retain/await request in runtime.action(cx, request).
```

`notification_permission()` queries without prompting. On macOS it returns
`Prompt`, `Granted`, `Denied` or `Unknown`. On Web it reads `Notification.permission`.
`request_notification_permission()` initiates a prompt only when the host needs one;
call it directly in a user event, especially on Web. Denial is `Ok(Denied)`, so an
app can keep working. Changing permission later requires the host's settings UI.
Do not request permission from render, startup or a background task.

`show_notification` returns `NotificationOutcome::Submitted` or `Denied` inside a
`platform::Request`. macOS awaits authorization before submitting. On Web posting
never prompts; without an existing grant it returns `Denied`. Windows and Linux
return `PermissionState::Unknown`, because native delivery exposes no permission
query; OS suppression cannot be reported as `Denied`. `Submitted` acknowledges
submission, never guarantees a banner or delivery. Focus modes, administrator
policy, user settings, and missing notification infrastructure can suppress it.
Operational failures that the adapter can observe are typed `platform::Error`s;
the native delivery API itself has no success receipt.

## Activation and lifetime

Rake initializes the process identity and installs the notification response handler
after UI initialization, before application setup or mounting views. Clicking a
notification activates the native app (best-effort browser focus on Web), then puts
its `activation_url` into the same shared inbox as host URL activation. Rake owns
this single native callback slot; replacing it through the low-level host API
bypasses this integration. Do not combine it with another system-notification
callback owner.

Keep one application-owned consumer of `platform.next_open_url()`. Validate the
scheme, host, path and arguments, then navigate or dispatch a typed application
command. There is no automatic URL-to-command execution. Custom application URLs
are payloads in this path and do not require registering an external URL scheme.
The existing inbox holds 32 pending activations and drops its oldest on overflow.

Tags identify replacements in one running application. A click consumes its payload;
late duplicate responses and dismissed/unknown tags are ignored. Up to 128 distinct
notification payloads may be pending. Further posts return `Unavailable`; replacing
an existing tag still works. Call `dismiss_notification(tag, cx)` to release a tag
and request host removal, even if its banner has already disappeared. Dismissal is
best effort and does not erase host-managed history. Tag/title/body are limited to
128 bytes/1 KiB/16 KiB respectively; tag and title must be nonempty. URL payloads
must be absolute and no longer than 16 KiB.

Payloads and response callbacks live in the running process/tab. Notifications
remaining in a system center after exit do **not** relaunch or restore Rake payloads.
On Web, dropping the last application service closes its notifications and detaches
callbacks. Native host lifetime is the application lifetime. Closing a window alone
does not necessarily stop either host. Server push, service-worker notifications,
rich actions and inline replies are outside this API.

## Platform requirements

| Platform | Requirements and limits |
| --- | --- |
| macOS | Launch an actual `.app` bundle with `CFBundleIdentifier`; ship a signed bundle with a stable identifier. The bundle identifier, not `App::new` alone, identifies authorization. An unpackaged `cargo run` executable returns `Unsupported` before touching the authorization center. First posting/requesting permission can prompt; denial is a normal result. No notification entitlement or usage-description key is added. Sandboxed distributions must follow their own signing policy. |
| Windows | Use the app identity installed by the distribution pipeline and keep it stable. Rake passes its application ID to the host at startup. The host uses package identity when present, otherwise a per-user application identity. A running process and an available toast service are needed for callbacks; cold activation is not implemented. Permission/suppression status is `Unknown`. |
| Linux | Requires a graphical session with a notification daemon on the session D-Bus. The daemon must support default-action callbacks for clicks; not every daemon does. Replacement, focus and dismissal depend on the desktop. Headless sessions may silently suppress native delivery. Permission status is `Unknown`. |
| Web | Requires HTTPS or a trustworthy localhost origin, a supported `Notification` constructor, permission, and a running tab. Request permission directly from a click/key event; embedding/cross-origin policy may deny it. Some mobile browsers only support service-worker notifications and reject this constructor: an explicit host error is returned. No background delivery after the tab exits. Browser focus is best effort. |

Use the same identifier in `App::new` and bundle metadata. Rake's bundle/distribution
pipeline supplies macOS bundle metadata/signing and Windows installer identity;
this API does not generate identities or bypass host authorization.
See [packaging](packaging.md) and [desktop distribution](distribution.md).

## Example and verification

The [Foundation example](../examples/foundation/src/lib.rs) has separate permission,
show and dismiss buttons, displays errors/results, and consumes the shared URL inbox
to display the clicked export payload. The same factory builds native and Web apps.

```sh
cargo run -p rake-app-cli -- bundle --target desktop --package rake-foundation
cargo run -p rake-app-cli -- bundle --target web --package rake-foundation
```

Run each native bundle on its matching host; serve the Web bundle from HTTPS or
localhost. Click **Allow OS notifications**, verify the displayed permission, then
**Show OS notification**. Clicking its body should foreground the app and display
`Notification opened export /42`. Repeat a tag to check replacement; dismiss it and
confirm a late click does not deliver another payload. Deny permission on a fresh
app/origin and verify a normal `Denied` result. With `cargo run -p rake-foundation`
on unpackaged macOS, expect `Unsupported` and no crash or permission prompt.

Automated checks:

```sh
cargo test -p rake-app --no-default-features --lib --test platform
cargo test -p rake-app --features view-test-support --lib
node scripts/test-platform.mjs
```

These exercise authorization denial, unpackaged macOS guards, native delegation and
response routing with the headless test host, and the real browser adapter against
a deterministic Notification host. They do not prove a real OS banner, browser
permission UI or packaged click on every desktop; use the manual procedure above
for release verification.
