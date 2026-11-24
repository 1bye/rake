# Contributing

Agents and people follow the same rules. Read [AGENTS.md](AGENTS.md) and its Rust workflow
guides before changing Rust code.

## Before you open a pull request

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 scripts/check-gpui-pins.py
python3 scripts/check-spacing.py
```

The scripts need Python 3.11 or newer. `check-spacing.py` runs a pinned development-only
nightly and doesn't change the stable toolchain. Some release tests need OpenSSL rather than
LibreSSL.

[`check.yml`](.github/workflows/check.yml) holds the complete native, Wasm, DevTools and
documentation checks. It currently runs only when started by hand.

## Application crates

Use [`rake-core`](crates/rake-core) for portable runtime and routing code, and
[`rake-gpui`](crates/rake-gpui) for the GPUI hosts, views and window bindings.
Examples and consumer tests use the [`rake-app`](crates/rake) facade. See the
[development guide](docs/development.md#workspace-crates) for crate-specific checks.

## Project conventions

- Compiling isn't proof that something works on a platform. Say what you actually ran.
- GPUI, GPUI Base and GPUI Kit pins move together. See the [integration guide](docs/gpui-kit.md).
- Put user-facing guides in `docs/`, contributor material in `docs/internal/` and one-off
  evidence or plans in `.agents/`.

## Internal documents

- [Release qualification](docs/internal/qualification.md)
- [Native distribution tests](docs/internal/distribution-native-tests.md)
- [Fieldnotes browser checks](docs/internal/notes-browser-testing.md)
- [Test coverage](docs/internal/test-coverage.md)
- [DevTools coverage](docs/internal/devtools-coverage.md)
- [Distribution planner](docs/internal/distribution-planning.md)
- [Original API design draft](docs/internal/api-design.md)
- [Legacy inspector prototypes](docs/internal/legacy-inspectors.md)
- [Future: GPUI variations](docs/future/gpui-variations.md)
