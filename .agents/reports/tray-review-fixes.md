# PR #23 review fixes

## 1. Merge main

Merged origin/main at 923f49a without rewriting the feature branch. Preserved
rake-app's crate name/library import and exact released dependency conventions;
new rake-tray is 0.1.0-alpha.3 with crates.io publish metadata and licenses. The
tray example remains unpublished. Tray CI/docs use the renamed package.

Validation before subsequent fixes: tray all-target build, rake-tray tests,
rake-app tray/view-test-support unit tests, tray example and renamed consumer
checks passed with installed Rust 1.98.0. No toolchains/packages were installed.
