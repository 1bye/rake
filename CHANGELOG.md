# Changelog

## SDK 0.1.0-alpha.6 and CLI 0.1.0-alpha.7 — 2026-11-23

- Added the app-scoped native keychain secrets API, owned global shortcut
  registrations, and single-instance activation forwarding. Window-state
  persistence now drains pending writes with a bounded quit deadline.
- Added live system appearance palettes, external file drops on desktop and Web,
  and native file icons with a bounded cache.
- Added bounded private application logging and `rake ctl logs` for reading and
  following logs. Rake redacts user content from its own runtime diagnostics.
- Added styled selectable paragraphs and accessible inline and standalone links
  with keyboard activation and drag selection. Published Rake uses the stock
  GPUI Base 0.9.0 window selection. Its Root trims leading and trailing
  whitespace on copy and binds Control-C on Web. The workspace Root also
  preserves whitespace and binds Command-C on Web.
- Added experimental signal-based function components and the `locale` and
  `locale-icu` prototypes for Fluent messages, locale switching, and ICU number
  and date formatting. Inspection can find controls by their message keys.
- Improved CLI Windows inspection and source opening, development profile
  diagnostics, and view syntax support. Registry scaffolds pin SDK alpha.6.
- Released the first `rake-shortcuts` package along with the changed supporting
  crates at alpha.5.

## CLI 0.1.0-alpha.6 — 2026-11-14

- Pins Blaze rake.3, fixing missing dependency manifests such as `hdrhistogram`
  when `--live --cdp` enables launch-specific devtools features. No app-level
  `devtools` forwarding feature or manual lockfile workaround is needed.
- Generated registry apps use the tested SDK alpha.5 independently of the CLI
  version. The SDK and supporting crates are unchanged.
- Shows native build progress with named steps and elapsed times.

## 0.1.0-alpha.5 — 2026-11-13

- CLI: enabled `rake dev --live --preview`; previews follow live patches and reconnect
  after checkpointed restarts.
- SDK: fixed devtools inspection of virtual-list rows laid out during prepaint,
  avoiding the `captured boundary cache` panic and retaining row identity and text updates.
- Generated apps use the matching alpha.5 SDK. Unchanged supporting crates remain
  pinned to alpha.4.

## 0.1.0-alpha.4 — 2026-11-13

- Added experimental `rake dev --live` for Apple Silicon Macs, with automatic
  installation of the pinned Blaze rake.2 bundle, body patches and checkpointed
  restarts. Unregistered state resets on restart; use `--require-state` to block it.
- Kept generated apps and all Rake crates on the same registry release. The exact
  supported Blaze runtime is packaged as `rake-blaze-live` with its upstream licenses.
- Added the Rust development supervisor, Windows control endpoints, native menu and
  window-state support, and clearer development output.


- Added application-owned OS notifications through platform services, explicit
  authorization and denial handling, browser notifications, dismissal and click
  payloads in the shared URL activation inbox. See [notifications](docs/notifications.md)
  for platform requirements and the Foundation example.

- Native GPUI `App` now records bounded, private panic records locally by default,
  including development runs. Delivery remains opt-in and Rake never uploads.
  Default setup errors warn and continue. Use `App::without_crash_reports()` to
  disable setup, or `App::crash_reports(rake::crash_config!(), sink)` to replace
  defaults and opt into delivery. The last builder call wins. Web and headless
  behavior, record format and privacy defaults are unchanged. See
  [crash reports](docs/crash-reports.md) for paths, limits and hook chaining.
