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

## P2: native ownership and registration

Implemented application-owned native resources with weak collection/registration
handles, generation-checked events, bounded ingress with explicit overflow,
revision-aware asynchronous updates and deterministic shutdown. AppKit and Win32
use private direct adapters; Windows owns a hidden top-level callback HWND per
item, stable per-item GUID and TaskbarCreated restoration. Linux uses KSNI's
async-io service, host observations and owned shutdown requests. The Rake facade
adds App::tray and RuntimeContext::trays without requiring user-owned globals.

Validation: 7 core/consumer tests, Rake tray-feature check and all-target Clippy
passed. The native AppKit run-loop probe passed with two actual status items,
geometry, updates, hide/show, remove/recreate and weak-owner shutdown. Backend
checks passed for Windows MSVC and Linux GNU. WASM is an explicitly unsupported
native backend; its portable contracts compile. These are compilation results,
not Windows/Linux GUI qualification.

## P3 — native menus and foreground dispatch

Implemented AppKit NSMenu/target/delegate ownership, Win32 HMENU tracking with focus
return, and SNI DBusMenu projection. Menus include nesting, check/radio/disabled/
hidden items, native icons on AppKit/SNI, and guarded Quit. Tracking defers AppKit
rebuilds. Scoped menu revisions reject stale callbacks after an update. Foreground
handlers run after native callbacks return and without registry borrows. Typed
commands use the existing scheduler without consuming transport retry keys.

Validation: core 8 tests; facade command regression (300 activations, runtime
release); facade all-target Clippy with tray; WASM core Clippy. Actual AppKit
NSMenu target/action probe passed, including revision change. Windows/Linux native
execution remains unavailable. Accelerators, Windows menu bitmaps, and extended
menu presentation continue in P5; lifecycle callbacks are capability-dependent.

## P4 — repeatable standalone main window

Added App::main_window(options_factory, mount), optionally combined with background
startup. It retains one scheduler/runtime, shares the original services, focuses an
existing window, and recreates after KeepRunning close. Failed mounts remove the
failed window and remain retryable. Existing root/routed close contracts remain;
routed navigation's reveal/select already activates its WindowHost.

Validation: GPUI TestPlatform regression exercises failed first mount, focus without
remount, close/reopen with identical runtime identity, and shutdown rejection. This
is scheduler/host-adapter coverage, not native focus qualification. Browser factory
configuration is explicitly rejected.

## P5 — native appearance and menu extensions

AppKit now supports content/square/point lengths, text-only/combined items,
alternate images/title, appearance-only versus interaction disable, accessible
name/help, autosave identity and KVO user-removal events. Managed removal Quit is
separate from native unguarded termination. Read-only metrics expose actual native
visibility. All raster representations become native image representations.
Menus include native shortcuts, alternates, indentation, tooltips, state images,
14+ badges/palette/selection layout and 14.4+ subtitles with selector gates.

Windows menus include owned DIB images, explicit mnemonics/default items, column
breaks and right-order cascading. Icons select representations for native DPI and
refresh for display/settings changes. SNI maps rich plain-text tooltips (escaped
body), title/category/status, theme paths, overlay/attention images and movie,
menu shortcuts/mnemonics, and registration-time sandbox bus-name policy. Host
shortcut interpretation remains conditional.

Validation: core 9 tests; AppKit native probe (two icons, transient text-only item,
explicit length, target/action and cleanup); facade tray/view-test-support
all-target Clippy. Windows/Linux foreign compile and Clippy are tracked separately
from desktop qualification. Native attributed text and specialized custom drawing
remain advanced dispositions; no blanket feature claim is made for these hooks.
