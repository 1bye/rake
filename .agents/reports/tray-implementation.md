# Tray implementation evidence

Plan: ../plans/tray-icons.md. Implementation branch: feat/tray-icons.

## P0: backend adoption and native gates

The implementation is isolated from the planning checkout. Source baseline is
752b7aa (origin/main). Rust 1.98 and macOS SDK 26.6 are installed. Rust targets
include macOS arm64, Windows MSVC, Linux x86_64 and browser WASM; installed targets
alone do not establish native desktop or cross-linking support.

Inspected installed upstream sources: tray-icon 0.26.1 (MIT/Apache, MSRV 1.90),
muda 0.21.2 (MIT/Apache, MSRV 1.90), ksni 0.3.7 (Unlicense, MSRV 1.80).
Tray-icon exposes native AppKit/Windows handles, but uses process-global event
handlers and its event contract omits several native semantics. Its default
Linux backend adds GTK; KSNI supports async-io without Tokio or GTK.

Adoption direction: isolate native resources, protocol models and event ingress
in a GPUI-independent rake-tray crate. This is a concrete platform/build boundary:
it permits native adapter checks and headless contract tests without compiling
GPUI for a foreign host, and lets the Rake facade own application integration.
Use private AppKit/Win32 adapters and KSNI with default features disabled.
Keep native library types out of Rake's shared API. Do not add Ayatana or a
second GUI event loop. Use public OS APIs for extensions rather than capping
features at a wrapper's event model.

Native gates remain explicit: only macOS can be exercised on this machine.
Windows/Linux GUI journeys require their actual desktop hosts. macOS 27 expanded
interface symbols are absent from the installed SDK. Newer native symbols must
be runtime/compile-time gated; no unsupported API is reported successful.

Phase-specific implementation, command results and limitations are appended as
work proceeds. This report does not claim P0's foreign native gates passed.

## P1: portable contracts

Added the GPUI-independent rake-tray crate with validated image ownership,
stable-keyed bounded menus, semantic input/geometry contracts, feature-specific
capabilities and typed failures. Menu dispatch rechecks hidden/disabled ancestors.
Native registration ownership follows in P2 so no fake backend success is exposed.

Validation: cargo test -p rake-tray (3 external-consumer tests) and
cargo clippy -p rake-tray --all-targets -- -D warnings passed.
