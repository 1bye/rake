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
