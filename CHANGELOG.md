# Changelog

## Unreleased

- Native GPUI `App` now records bounded, private panic records locally by default,
  including development runs. Delivery remains opt-in and Rake never uploads.
  Default setup errors warn and continue. Use `App::without_crash_reports()` to
  disable setup, or `App::crash_reports(rake::crash_config!(), sink)` to replace
  defaults and opt into delivery. The last builder call wins. Web and headless
  behavior, record format and privacy defaults are unchanged. See
  [crash reports](docs/crash-reports.md) for paths, limits and hook chaining.
