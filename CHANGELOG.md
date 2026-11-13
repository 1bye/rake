# Changelog

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
