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

Target overrides apply after common/platform settings. Format overrides apply
next; explicit CLI format selection replaces the requested output list. Arrays
replace, while update public-key maps merge by provider. A format override cannot
change the format list. Use `--target-triple` to select a target layer. Rust
targets are parsed and must match `--platform`.

```toml
[package.metadata.rake.bundle.target_overrides.x86_64-pc-windows-msvc]
formats = ["msi"]

[package.metadata.rake.bundle.format_overrides.msi]
package_version = "1.2.301"
```

MSI/MSIX prereleases require an explicit numeric `package_version`; dropping a
SemVer prerelease suffix could otherwise prevent a later upgrade. MSI compares
three bounded numeric fields; MSIX uses four. Debian/RPM prereleases use `~`.
Arch prereleases require an explicit mapping verified using `vercmp`. macOS
prereleases require a monotonic numeric `build_number`.

Schema 1 native bundling now runs in Rust. `rake bundle --format app --debug`
uses an isolated worker, copies Cargo output, stages inputs, and writes
`distribution.json`. `--native` opts a legacy configuration into that executor.
Legacy commands and `--target web` retain their existing implementation.

Add this build script for link-time resources, including Windows icons/version:

```toml
[build-dependencies]
rake-build = { path = "../../crates/rake-build" }
```

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    rake_build::build()
}
```

A PNG or SVG icon generates PNG/ICO variants; the Mac engine generates ICNS.
The SVG renderer accepts vector shapes without external image references.
Windows custom manifests replace resource #1 on a staged PE after linking;
start from `crates/rake-dist-windows/templates/default.manifest.xml` to retain GPUI's
DPI, compatibility and heap settings. `rake-build` adds no competing manifest.
The SDK's `mt.exe` validates and reads back the embedded resource.

At runtime, `rake::resources!()?` finds the signed installation descriptor and
resources beside the installed executable, independently of the working directory.
Use `.path("assets/file.json")?` or `.helper("ffmpeg")?`; helpers remain private
under the application's resource root. Development builds without a descriptor
return `ResourceError::Unbundled` through `rake::resources::Error`.

Native components use exact Rust targets. Helpers may refer to a prepared
component instead of a package-relative file:

```toml
[package.metadata.rake.bundle]
schema_version = 1
sidecars = [{ name = "ffmpeg", component = "media", path = "bin/ffmpeg" }]
components = [{ name = "media", target = "aarch64-apple-darwin", source = { type = "local", path = "vendor/media" }, licenses = ["LICENSE"] }]
```

HTTPS sources use `{ type = "https", url = "https://…", sha256 = "64 lowercase
hex digits", archive = "tar_xz" }`. ZIP and tar_gz are supported too; `file`
places a single downloaded file at `payload`. Downloads are size bounded and
verified before entering the content cache. Extraction rejects traversal, unsafe
links and special files, and limits file count/expanded size. Helpers/native
libraries must contain the selected CPU architecture.

`directory` deploys a private runtime; `gstreamer` additionally selects explicit
plugins from `lib/gstreamer-1.0`. Include its private scanner under
`libexec/gstreamer-1.0`. `Resources::gstreamer_command` sets plugin/scanner paths
for the child without changing the parent process environment. macOS `framework`
recipes preserve relative framework symlinks and expose `Resources::framework`.
All non-system macOS load commands must resolve within the staged inventory.

Each distribution includes `components.lock.json`. Check it in as
`rake-dist.lock` to reject changes to resolved inputs; `--locked-components`
requires that lock. `--offline-components` prohibits component downloads and
uses only verified cached bytes. Native installer engines can still require their
own native tools; this flag applies specifically to component resolution.

macOS App/ZIP/DMG finalization now signs all native payloads inside out, verifies
signatures, and optionally requires notarization and stapled tickets. Set
`macos.signing_identity` and `macos.notary_profile` to existing local identity /
Keychain profile names, or pass `--sign-identity` and `--notary-profile`.
`--trusted` refuses missing release credentials. Ordinary development bundles
use ad-hoc signing. Ticket/signature checks run again after final copying.
`macos.universal = true` builds both Rust targets and combines them with lipo;
helpers and native libraries must also contain both slices.

Windows signing uses `windows.certificate_thumbprint` from the existing
current-user My certificate store, with `windows.timestamp_url` for trusted
releases. The signer verifies Authenticode after signing. MSIX requires a
certificate-matching `windows.msix.publisher`; an optional full
`windows.msix.manifest` must preserve generated identity and executable fields.
Velopack uses pinned vpk 1.2.161 and selects the matching Rake SDK feature for its
build variant. `#[rake::app]` runs installer startup ahead of the app factory;
manual entry points must call `rake::early_startup()` first. Automatic SDK
application of downloaded updates on startup is disabled so restart can be guarded.

Generated descriptors and multi-artifact paths use forward slashes on every host.
Relative native Windows separators are normalized before confinement checks;
traversal and drive/UNC prefixes remain rejected. Manifest root architecture is
resolved from the staged PE header; dependency architecture wildcards are retained.
