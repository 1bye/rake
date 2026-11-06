# PR #23 review fixes

## 1. Merge main

Merged origin/main at 923f49a without rewriting the feature branch. Preserved
rake-app's crate name/library import and exact released dependency conventions;
new rake-tray is 0.1.0-alpha.3 with crates.io publish metadata and licenses. The
tray example remains unpublished. Tray CI/docs use the renamed package.

Validation before subsequent fixes: tray all-target build, rake-tray tests,
rake-app tray/view-test-support unit tests, tray example and renamed consumer
checks passed with installed Rust 1.98.0. No toolchains/packages were installed.

## 2. Use unmodified upstream ksni

Removed vendor/ksni and its workspace patch. Linux uses public Handle::update,
is_closed and watcher callbacks. Receipts acknowledge local model processing;
upstream's hidden per-signal export errors and incomplete whole-bus-disconnect
observation are now explicit limitations. An upstream proposal is saved locally;
no upstream PR/issue was opened.

Validation: foreign Linux check and isolated SNI probe passed. Also packaged
rake-tray and checked the unpacked crate as a standalone Linux consumer outside
this workspace, so no workspace patch/path overrides could satisfy its build.

## 3. Optional trays do not block application startup

Validate duplicate IDs/shared configuration up front. Required operational
failures still roll back and fail startup; optional creation failures now leave
recoverable items to their backend or skip them while keeping the window/runtime
alive. Diagnostics reach the same foreground on_tray_event handlers. A missing
optional host is skipped and reports generation-zero startup diagnostics.

A feature-isolated failing native boundary tests the real registry/driver and GPUI
TestPlatform window host: Wait leaves the main window open and delivers an event;
Require records AppError::Tray and requests application UI shutdown. A missing
optional host is also covered. No native desktop behavior is inferred from these
doubles. Focused host tests, core test-support tests and facade Clippy passed.

## 4. Wait for initial Windows shell registration

Initial NIM_ADD failure under Wait retains the hidden broadcast HWND, native
resources and offline item. TaskbarCreated reuses the same add/version-4 sequence;
Require returns the error. Offline updates retain the latest cache for recovery.
Version-negotiation failure removes the partial legacy-format registration, and
callbacks are ignored while offline. Host-unavailable events also log a diagnostic.

Validation: cross-platform scripted Shell seam tests cover initial Wait failure,
recovery, Require failure and negotiation rollback. Core tests and foreign Windows
Clippy passed. Actual login/Explorer desktop qualification remains open.

## 5. Derived Windows GUID fallback

Retry a rejected derived GUID once using HWND/uID, and retain the selected identity
mode through recovery and every subsequent Shell operation. NIF_GUID is cleared
and guidItem is zero in fallback mode, including rectangle queries and balloons.
Explicit GUIDs are never replaced. Native diagnostics report identity fallback or
an offline explicit GUID; persistent identity capability is conditional.

Validation: scripted tests cover derived retry/mode retention, explicit rejection,
no-host retries preserving derived identity, and version-failure cleanup under the
fallback identity. Core tests and foreign Windows Clippy passed. Actual path-bound
GUID and Explorer behavior remain subject to Windows desktop qualification.

## 6. Keep tray command bindings chainable

tray_command now returns Self. Serialization failures retain their CommandError
inside the existing deferred configuration-error slot and surface at App::run;
route transitions preserve the first error. Updated the workbench and guide.
Documented the authoritative current menu model and the shared rake-menu/rake::menu
extraction point for #13; no menu-model refactor or issue mutation was performed.

Validation before commit: chainability/deferred-error/route-transition tests,
example compilation, focused facade Clippy and workspace Clippy passed. The final
post-commit verification matrix is recorded in the PR description.
