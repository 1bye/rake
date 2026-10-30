# Distribution planning

The new Rust planner validates distribution configuration without invoking native
packagers, building an app, signing, downloading or publishing. Native execution
and update adapters remain in implementation; a valid plan is not a claim of
native platform validation.

```sh
cargo run -p rake-cli -- bundle --plan --package rake-foundation --json
cargo run -p rake-cli -- bundle --plan --platform windows --package rake-foundation --json
cargo run -p rake-cli -- bundle --plan --schema --json
```

Use `--manifest-path` for another workspace. Cargo package selection is limited to
workspace members and requires `--package` for ambiguous workspaces. Metadata is
read using locked, offline Cargo resolution. The last command emits the editor
schema without invoking Cargo metadata.

Configuration lives under `package.metadata.rake.bundle`. Omitting
`schema_version` retains the legacy host layout defaults; version 1 defaults to
DMG, NSIS and AppImage on their respective platforms. The CLI returns
`native_validation: "pending"` because native capability/trust checks are separate.
Existing bundle commands without `--plan` still use the established pipeline.

The planner validates the examples in the distribution implementation plan,
relative input/destination paths, stable identity, semantic versions, duplicate
formats/component names, format/platform compatibility, public update settings
and MSI/MSIX numeric limits. It derives update ownership from the installation
format. DEB/RPM/Arch/Flatpak updates remain external package-manager operations;
MSIX uses App Installer, Velopack uses its own package family, and conventional
NSIS/MSI installers remain external/manual until a compatible updater is adopted.

The portable `rake-dist-core` crate has no packager, native-tool or GUI dependency.
Its optional `schema` feature enables schema generation for the host-side
`rake-dist` crate. That crate resolves Cargo package metadata and provides the
public schema. Native tooling must pass the independent adoption gate before
being added to this execution path.
