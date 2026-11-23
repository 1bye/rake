# SDK alpha.6 and CLI alpha.7 release — 2026-11-23

## Scope and source

Published twelve packages: SDK `rake-app 0.1.0-alpha.6`, CLI
`rake-app-cli 0.1.0-alpha.7`, and ten supporting crates at alpha.5, including
the first `rake-shortcuts` release. Unchanged supporting crates stay at alpha.4.

Earlier releases published fewer crates because supporting crates had not
changed (alpha.5: SDK and CLI; alpha.6: CLI only). Since alpha.6, diagnostics,
protocol, client, view syntax, macros, UI, native and dist changed in source,
and the SDK depends on those changes. `rake-http` has no source change but moves
with its exact `rake-diagnostics` and `rake-protocol` pins.

Release source: `68e81720` (merge of PR #109, release prep), from a clean clone
of `main`. Uploaded with `cargo publish --locked -p <crate>` in dependency order.

## Public packages

All times are UTC on 2026-11-23.

| Package | Version | Published | Crate SHA-256 |
| --- | --- | --- | --- |
| [rake-diagnostics](https://crates.io/crates/rake-diagnostics/0.1.0-alpha.5) | 0.1.0-alpha.5 | 16:23 | `3bd671998cc1e5c60c47dd58a99bded2ed2a6666941a9c80269556e91e797bcf` |
| [rake-protocol](https://crates.io/crates/rake-protocol/0.1.0-alpha.5) | 0.1.0-alpha.5 | 16:23 | `96c475e6d7f1888866d0f7c990f213b5ae71d84f0a948540a3a31ef8292df908` |
| [rake-http](https://crates.io/crates/rake-http/0.1.0-alpha.5) | 0.1.0-alpha.5 | 16:24 | `a72fe163f15d060fd4ecdeb41e11cf5487d1b04ed68c2a52a3ca11c35f7bcd57` |
| [rake-client](https://crates.io/crates/rake-client/0.1.0-alpha.5) | 0.1.0-alpha.5 | 16:24 | `9cd8536957a69e414c566e676c209ce7685024a8123fee2d8d34dcf7dec5a4c1` |
| [rake-view-syntax](https://crates.io/crates/rake-view-syntax/0.1.0-alpha.5) | 0.1.0-alpha.5 | 16:24 | `e4b1a2c6416e058946c7f427deb8b0f8c92efaab331c127481cb91809b34b8d6` |
| [rake-macros](https://crates.io/crates/rake-macros/0.1.0-alpha.5) | 0.1.0-alpha.5 | 16:24 | `dc952e5b5a8a3ce37d087d3169d4ea8f2c21bf1ebf6400265864325331434136` |
| [rake-ui](https://crates.io/crates/rake-ui/0.1.0-alpha.5) | 0.1.0-alpha.5 | 16:26 | `24ff6b63e700aeaa1949b84247a0e2db6a79313a254b69d93dc6eb06cb124bdb` |
| [rake-native](https://crates.io/crates/rake-native/0.1.0-alpha.5) | 0.1.0-alpha.5 | 16:26 | `0172ca953f2ba964afe968eaf1820a89c40c10779d0991f705d8b423309fe626` |
| [rake-shortcuts](https://crates.io/crates/rake-shortcuts/0.1.0-alpha.5) | 0.1.0-alpha.5 | 16:26 | `375a8e7773a0f97083aece97b80cb553a68c42665b95a2f056abf86445b5f7ac` |
| [rake-dist](https://crates.io/crates/rake-dist/0.1.0-alpha.5) | 0.1.0-alpha.5 | 16:27 | `0bc3ecea27f3041a1111bc39eefe44580692d425fe19ff14952fb0a280be200a` |
| [rake-app](https://crates.io/crates/rake-app/0.1.0-alpha.6) | 0.1.0-alpha.6 | 16:28 | `3ed09913b80a5bd445e1aa1db14b33e68b4a0148608e9f9b18d9cd1d48aa6462` |
| [rake-app-cli](https://crates.io/crates/rake-app-cli/0.1.0-alpha.7) | 0.1.0-alpha.7 | 16:29 | `894cfdd545058ff35d0e21b800d8efb803d06e2f82d7205b2572330dffab7c82` |

## Checks

- Before upload, Cargo built each package from its packaged sources (the normal
  `cargo publish` verification), in dependency order against the newly
  published registry versions.
- After upload, each public crate was downloaded from static.crates.io and
  compared file by file with the verified package sources: all twelve match.
- PR #109 recorded the pre-release checks: workspace publish dry run, stock
  crates.io consumers of `rake-app` and `rake-ui` on macOS and
  `wasm32-unknown-unknown`, scaffold builds, tests, strict Clippy, and manual
  selection, copy and link checks through `rake ctl`.

## Notes

- `rake-ui` carries an Apache-2.0 adaptation of GPUI Base 0.9.0's selectable
  text element (`crates/rake-ui/LICENSE-GPUI-BASE`) instead of requiring the
  vendored Base APIs. Registry consumers use the stock Base Root: copy trims
  leading and trailing whitespace, and Web binds Control-C only.
- Upstreaming the Base APIs to longbridge/gpui-kit is prepared but not opened.
- The first two publish attempts were killed with the agent shell before any
  upload; the successful run used a detached `screen` session.
