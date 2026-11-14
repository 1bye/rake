Adds experimental native hot reload through `rake dev --live`. The Rust supervisor
drives the Blaze daemon; Rake polls on a 16 ms UI timer, redraws applied patches,
and uses checkpointed restarts when layouts change or the patch area fills.
Plain `rake dev` stays on Cargo. Native restarts proceed by default with a notice
for missing state; `--require-state` keeps the old process when state is unavailable.

## Run it

```sh
rake new hello
cd hello
rake dev --live
```

Apple Silicon macOS only. Live mode downloads and verifies the pinned
`1bye/blaze` `v0.20261030.1-rake.1` bundle; no global Blaze install is needed.
The generated manifest exposes `live`, `devtools` and `gpui-fast` aliases so a
fresh Cargo lock includes optional dependencies and compatible Taffy versions.
Only live/devtools are enabled for the live build; the alternate backend stays off.
For local review, use `rake new hello --rake-path /path/to/rake/crates/rake`.

The pinned fork bundle carries these engine fixes:

- WASI realpath within the macro package sandbox.
- Cross-file `span_join` returns `None`.
- `blaze.std::` symbol prefix and standard-library cache-key bump.
- `module_path!` expansion in symbol attributes.
- Canonical `CARGO` path in macro sandboxes.
- Real `rustc` for build-script probes.
- Missing AArch64 NEON intrinsic lowering.

The fixed matching macro server is bundled. Hybrid native-macro settings, the
64 MiB stack, `pf-no-simd` through `rake/live`, and binary-only builds are automatic.

## Limits

- Only registered checkpoints are restored; other fields reset with a notice.
- Older/custom apps may need the feature aliases documented in `docs/development.md`.
  The pinned engine can plan inactive dependencies absent from Cargo's lockfile.
- Larger macro bodies can still take seconds to reload. Edited static initializers reset.
- One earlier router remount at edit 30 remains unexplained; later soaks did not reproduce it.
- First full restarts can be slow (55 s in an earlier larger fixture).
- Linux and Intel Mac live mode are not supported. CI tests use fake engines; no
  real Blaze build is required in CI.

## Validation

- Rebased onto current main `8ba48c2c`; frame polling is folded into the UI-timer fix.
- 998 workspace tests; 6 live-runtime tests; 15 supervisor tests;
  4 installer tests, including wrong checksums and tampering.
- CI-style fmt, workspace Clippy, live-feature Clippy and spacing checks.
- Fresh generated app: live text/helper edits, field restart with a visible notice,
  patch after restart, and plain Cargo dev.
- Published-bundle View/Notes smoke edits; unseeded scaffold builds natively and for Wasm.

Details: `.agents/reports/pr53-mergeable.md`.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
