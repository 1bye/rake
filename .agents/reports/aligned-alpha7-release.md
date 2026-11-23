# Aligned 0.1.0-alpha.7 release — 2026-11-23

## Scope and source

Every published Rake crate now inherits `[workspace.package].version =
"0.1.0-alpha.7"` and pins its Rake dependencies at `=0.1.0-alpha.7` (PR #112).
Examples and test crates stay at `0.1.0`: inheriting a prerelease version made
macOS bundling require a monotonic build number.

Release source: `7692eb64` (merge of PR #112). Published with
`cargo publish --workspace --exclude rake-app-cli --locked`, which ordered the
21 packages itself. `rake-app-cli 0.1.0-alpha.7` was already published with SDK
alpha.6 and was not republished; it still scaffolds SDK alpha.6.

No API or behaviour changes since SDK alpha.6. GitHub release:
[v0.1.0-alpha.7](https://github.com/1bye/rake/releases/tag/v0.1.0-alpha.7).

## Public packages

All times are UTC on 2026-11-23.

| Package | Published | Crate SHA-256 |
| --- | --- | --- |
| [rake-app](https://crates.io/crates/rake-app/0.1.0-alpha.7) | 19:17 | `59d3f04ed02a74af9c73fd8ca7816ebca34201abb5fe667121ff1dcde86b7d9e` |
| [rake-autostart](https://crates.io/crates/rake-autostart/0.1.0-alpha.7) | 19:16 | `d976af42da2a0cd9b4fe45534f23a7f6345931a15d186cc43c3eb86655aca47e` |
| [rake-blaze-live](https://crates.io/crates/rake-blaze-live/0.1.0-alpha.7) | 19:16 | `131b3ec462d659ce2e2ce6d59e27c19e0ab925c3c0c7aea9da4a8ad5e65843cb` |
| [rake-build](https://crates.io/crates/rake-build/0.1.0-alpha.7) | 19:16 | `ee5a73e3a39d20c0e19e36f6b8c913304809086f0b70f674c94ee008daf01660` |
| [rake-client](https://crates.io/crates/rake-client/0.1.0-alpha.7) | 19:16 | `dd3906eb7fdb93825f50fb71ec8a14fab1a99d471a1bfaf413556a23034e5251` |
| [rake-diagnostics](https://crates.io/crates/rake-diagnostics/0.1.0-alpha.7) | 19:16 | `7e7fe2f1d541e07d0271108ae905be1e39cfa6da4fb15376d9cddf8a4b9905b7` |
| [rake-dist](https://crates.io/crates/rake-dist/0.1.0-alpha.7) | 19:17 | `2dd0850b1781224156980e1c089e071b54351e18369aa6f94dedfa165ef6f325` |
| [rake-dist-core](https://crates.io/crates/rake-dist-core/0.1.0-alpha.7) | 19:16 | `e9ff62b3db19ffc270e5da14945807511292c492d863ed8f32d212453d439dd0` |
| [rake-dist-linux](https://crates.io/crates/rake-dist-linux/0.1.0-alpha.7) | 19:16 | `40a7f02e7d6bf7f263f4e7c0332d7ed3757f8424dbf8bc87d1e2987edf4c5f1f` |
| [rake-dist-macos](https://crates.io/crates/rake-dist-macos/0.1.0-alpha.7) | 19:16 | `271deaa9c3d88078a755a00dca777f833aa6d86c3953eb83d5284df16593eed0` |
| [rake-dist-windows](https://crates.io/crates/rake-dist-windows/0.1.0-alpha.7) | 19:16 | `1de23dac6462d3c23fc868863507007ae8aeff495eaa9a72b118418ed41d2ea8` |
| [rake-http](https://crates.io/crates/rake-http/0.1.0-alpha.7) | 19:16 | `48dc8da9b943e63fbf4e6aecfa107e205be4789996efc9e0c05b13e80ddeab59` |
| [rake-macros](https://crates.io/crates/rake-macros/0.1.0-alpha.7) | 19:16 | `d1b397b696fbd7d34544bf26896a4a919a6d0b2c283467faf7137c66dc0e3e01` |
| [rake-native](https://crates.io/crates/rake-native/0.1.0-alpha.7) | 19:16 | `3ad482c13c6e46f4571f50e23bed36e6d8d8d0dd7f16762a1b063a60cf65a098` |
| [rake-protocol](https://crates.io/crates/rake-protocol/0.1.0-alpha.7) | 19:16 | `c9bc7618ff1d8ea118c6c74f482406aaa609fdf010730edb759e3d5c32da822a` |
| [rake-resources](https://crates.io/crates/rake-resources/0.1.0-alpha.7) | 19:16 | `cc3966d1b0de365b81042c34a5f73792f4c97e4bc2c194f3a3435392bbc028ef` |
| [rake-shortcuts](https://crates.io/crates/rake-shortcuts/0.1.0-alpha.7) | 19:16 | `c460102f5bd92c637397ada2359779bb95cd41b8f359741ce10b38f8daedd800` |
| [rake-tray](https://crates.io/crates/rake-tray/0.1.0-alpha.7) | 19:16 | `85264859c4d7d4e904870d9cbf5775d4ebdbd064e16ad11328c6d11b65f6413c` |
| [rake-ui](https://crates.io/crates/rake-ui/0.1.0-alpha.7) | 19:16 | `775e69c8dfde329ebd54be8208bdd8dd96de07db908dac6c283707308859ac4f` |
| [rake-updates](https://crates.io/crates/rake-updates/0.1.0-alpha.7) | 19:16 | `f824133bb3e3314bc7a53f3f6caf9db17d9a9d3bb1b2ee89fa01ce31830b2ce9` |
| [rake-view-syntax](https://crates.io/crates/rake-view-syntax/0.1.0-alpha.7) | 19:16 | `1890fc3f6857ea2dd79f740b7c7482e75cea4676dd4b1e6d961b1dde921a69df` |

## Checks

- `cargo publish --workspace --exclude rake-app-cli --dry-run --locked` passed
  before merge; PR #112 CI passed (13 checks).
- Cargo built each package from its packaged sources before upload.
- After upload, each public crate was downloaded from static.crates.io and
  compared file by file with the verified package sources: all 21 match.
