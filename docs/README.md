# Rake documentation

New here? Start with [Getting started](getting-started.md), then read
[what's supported](support.md).

## Build an app

| Guide | Covers |
| --- | --- |
| [Application shapes](application.md) | Standalone and routed apps, startup, shutdown |
| [Routing](routing.md) | Typed routes, tabs, history, guards, loaders |
| [Managed work](managed-work.md) | Tasks, activities and cancellation |
| [Guarded quit](guarded-quit.md) | Save/discard/cancel on exit |
| [Services](services.md) | Composing application services |
| [Storage](storage.md) | Typed persistent storage |
| [Native crash reports](crash-reports.md) | Private panic records and application-owned delivery |
| [Platform services](platform-services.md) | Files, clipboard, preferences, URLs |
| [Browser apps](web-applications.md) | Workers, offline bundles, deployment updates |
| [Fieldnotes](notes.md) | The complete reference app |

## Build UI

| Guide | Covers |
| --- | --- |
| [View syntax](view.md) | `rake::view!` reference, styling and themes |
| [Overlays and scrolling](../crates/rake/overlays.md) | Dialogs, popovers and scrolling; evolving overlay API |
| [GPUI Base and Kit](gpui-kit.md) | Base-backed controls, optional Kit, shared colors |
| [Native controls](../crates/rake-native/README.md) | AppKit views in GPUI layouts |

## Develop and debug

| Guide | Covers |
| --- | --- |
| [Source linting](lint.md) | Accessibility and loop IDs without building the project |
| [Development and reload](development.md) | `rake dev`, watched builds, checkpoints |
| [Chrome DevTools](devtools.md) | Connecting DevTools to a running app |
| [Element inspection](element-inspection.md) | Boundaries, picking, live style edits |
| Chrome panels | [Console](devtools-console.md), [Network](devtools-network.md), [Performance](devtools-performance.md), [Memory](devtools-memory.md), [Sources](devtools-sources.md) |
| [Native preview](dev-preview.md) | macOS browser preview prototype |

## Work with agents

| Guide | Covers |
| --- | --- |
| [Runtime control](runtime-control.md) | Commands, routes and logs from the CLI |
| [CDP agent client](cdp-agents.md) | Native UI automation, waits and screenshots |

## Ship

See the [changelog](../CHANGELOG.md) for unreleased behavior changes.

| Guide | Covers |
| --- | --- |
| [Packaging](packaging.md) | Archives and Web bundles |
| [Desktop distribution](distribution.md) | Installers, updates and metadata |
| [Releasing native apps](distribution-release.md) | Signing and publication |

Contributor material is listed in [CONTRIBUTING.md](../CONTRIBUTING.md).
