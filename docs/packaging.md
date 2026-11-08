# Packaging and distribution

> Windows and Linux, signing, notarization and installers aren't qualified in this alpha.
> See [what's supported](support.md).

This page describes the legacy macOS/Linux archive layout and the Web pipeline.
For native installers, updates, metadata, components and release publishing, use
[Desktop distribution](distribution.md). Schema 1 selects that workflow; legacy
native bundles retain their layout and JSON through the same Rust coordinator.

`rake bundle` uses the same package selection, Cargo build and Wasm binding generator
as `rake build`. It needs no running app, dev supervisor or runtime connection.

```sh
cargo build -p rake-app-cli
target/debug/rake bundle --target desktop --package rake-routing --json
target/debug/rake bundle --target web --package rake-routing --base-path /notes/ --routing path --json
target/debug/rake bundle --target web --package rake-routing --routing hash
```

Bundles default to the **release** profile, and `--debug` is for local packaging checks. `--manifest-path`, `--package`, `--bin`, `--features`, `--web-dir` and `--output` work as for builds.

Defaults write `dist/PACKAGE/desktop-bundle` or `dist/PACKAGE/web-bundle`, separate from build outputs at `dist/PACKAGE/desktop|web`.

Builds, bundles and the development supervisor run in Rust without Python. Web also needs the wasm32 target and a `wasm-bindgen` CLI exactly matching `Cargo.lock`.

Each output contains the unpacked artifact, archive, `bundle.json` with versions,
target, profile, signing/deployment status and file hashes, plus `SHA256SUMS` for
the archive. JSON command output includes absolute artifact/archive/manifest paths
and the archive SHA-256. No source directory paths or development credentials are
written into the public bundle manifest.

Publication uses a staging directory beside the output. A failed build, copy,
binding generation, signing or archive operation preserves the previous output.
An ownership marker prevents replacement of an unrelated directory or a build of
another kind. Symlink outputs and overlapping output/input trees are rejected.

## Metadata and assets

Configuration belongs to the selected package's Cargo.toml:

```toml
[package.metadata.rake.bundle]
name = "Notes"
identifier = "com.example.notes"
assets = "assets"
icon = "icons/notes.png" # Optional; .png or .svg, also .icns on macOS
build_number = "12"     # Optional; defaults to the numeric Cargo version
minimum_system_version = "13.0" # Optional macOS deployment declaration

[package.metadata.rake.bundle.web]
base_path = "/notes/"
routing = "path"
```

- **Display name:** defaults to the Cargo package name.
- **Identifier:** defaults to `dev.rake.PACKAGE`, with underscores replaced by hyphens. Set it to the application ID passed to `App::new` for a consistent OS and runtime identity.
- **Version:** Cargo's full version stays in the manifest and archive, and the numeric part supplies the macOS short version.
- **Build numbers:** one to three numeric components.
- **Minimum OS:** declares compatibility. It doesn't change Rust or linker deployment targets, or prove that dependencies support that OS.

Native apps use Rake's static Vermilion icon when `icon` is omitted, including
development apps launched with `rake dev`. Set `icon` to use your own design.

Web development and deployment bundles use the same SVG as a default favicon.
The native Rust `rake build --target web` path does not yet install this fallback;
see the [landing report](../.agents/reports/land-native-review.md) for this follow-up.
Declare a `<link rel="icon" href="...">` in your Web host document to override it; explicit empty favicons such as `href="data:,"` are also
preserved. The generated `assets/rake-icon.svg` follows the deployment base path
and offline asset layout.

Asset and icon paths are explicit, package-relative inputs. Files outside the package, symlinks, special files and output-name collisions fail. The asset tree is copied without following links, and static resource permissions are normalized.

Only declared asset trees and the Web host tree are copied, not the project or dev-session data. This stage packages resources; your app chooses how to decode or register them with GPUI. Runtime paths are:

| Target | Asset directory |
| --- | --- |
| macOS | `APP.app/Contents/Resources/assets/` |
| Linux | `application/share/IDENTIFIER/assets/`, relative to the bundle prefix |
| Web | `site/assets/`, served at `BASE_PATH/assets/` |

Native applications must resolve resources relative to their executable/bundle,
not the process working directory. Linux executables live in `application/bin/`;
macOS executables live in `Contents/MacOS/`. The routing example declares a small
JSON asset. Tests relocate the native archive and read a nested resource from an
unrelated working directory.

## Native artifacts

On macOS the artifact is a versioned `.app` with an executable, Info.plist, resources and an icon. PNG and SVG sources are converted to ICNS.

Plain non-system dylib dependencies are resolved from loader and executable paths and `LC_RPATH` entries, copied recursively into `Contents/Frameworks`, rewritten and signed before the outer app. Filename collisions and unresolved libraries fail. Use explicit framework and runtime recipes in schema 1 for external framework bundles and plugins. System frameworks and `/usr/lib` dependencies stay system-provided.

The default is an **ad-hoc signature**, verified with `codesign --verify --deep
--strict`. This supplies local bundle integrity, not Developer ID/notarization or
Gatekeeper distribution approval. For an explicitly selected installed identity:

```sh
target/debug/rake bundle --target desktop --package notes \
  --sign-identity "Developer ID Application: Example (TEAMID)" \
  --entitlements entitlements.plist
```

Identity signing uses a secure timestamp and Hardened Runtime for the main app, and entitlements are optional. Signing failure prevents publication. The ZIP is made with `ditto --keepParent` after signature verification, following Apple's [distribution signing](https://developer.apple.com/documentation/xcode/creating-distribution-signed-code-for-the-mac/) and [packaging guidance](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution).

Ordinary development bundles report `notarized: false`. Explicit trusted releases use an enrolled Developer ID identity and `--notary-profile`, then verify notarization and stapled tickets. See [release settings](distribution-release.md). App Store submission is a separate publishing adapter.

Linux produces a directory archive with `bin/` and `share/` resources. It relies on compatible system libraries and isn't an AppImage, distro package or installer. The layout is validated on macOS and Linux.

Schema 1 also produces Linux native packages, AppImage and Flatpak. Windows distribution commands run independently of Unix development-control IPC and support NSIS, MSI, MSIX and Velopack. macOS archives report the actual Mach-O architecture, and universal builds with matching native components use the schema 1 configuration.

## Web deployment

The `site/` directory and ZIP contain the Wasm module, matching JS bindings, host and loader, and declared assets. The ZIP has the site contents at its root.

The host contains `rake-web-config` metadata, which the runner applies before initial route decoding and uses for later History API writes. Path and hash routing both keep the prefix, including on Back, Forward and clearing the last tab. CLI flags override bundle Web metadata. Development builds keep their normal root host; `rake bundle` applies deployment settings.

`WebConfig` owns its prefix and is cloneable. Applications can also configure it
directly with `WebConfig::new().base_path("/notes/")?`; `uri_for` and
`destination_from_url` share the browser adapter's typed deployment codec.

**Base paths** begin and end with `/`, use ASCII letters, digits, `-`, `_`, `.` and `~` in nonempty segments, and reject dot segments and encoded, query or fragment syntax.

Rake adds `<base>`, rebases `/pkg/` and `/assets/` references in custom HTML, JS and CSS, and leaves the generated bindings' module-relative Wasm resolution intact. Custom root-relative application and API URLs stay yours to manage.

- Remove any custom `<base>` element before bundling.
- `pkg/`, `rake-web-config` and `rake-deployment.json` are reserved.
- Inputs containing development bootstrap or bridge metadata are rejected.

Upload **site contents** below the configured URL prefix, deploying the complete directory as one version so old bindings and new Wasm never mix. `rake-deployment.json` records routing, base, asset, MIME and cache requirements, and `nginx.conf.example` supplies path and hash host rules.

| Requirement | Rule |
| --- | --- |
| Path mode | App-shell fallback for destination URLs, and 404 for missing Wasm or asset files |
| Hash mode | No destination-path rewrite needed |
| MIME types | `.wasm` as `application/wasm`, JS with its JavaScript type |
| Caching | Revalidate index, loader, bindings and Wasm. Their filenames aren't content hashed, so immutable caching would mix releases |

No broker, dev bootstrap, inspector assets, token, reload checkpoint or connection
metadata is added to production output. A normal static server is sufficient.
Rake creates artifacts and host guidance; it does not upload to a hosting account.

## Verification

```sh
python3 scripts/test-bundle.py
cargo test -p rake-app --no-default-features --test web_deployment
```

The packaging suite uses real tiny Rust/Wasm apps, platform codesign/archive tools,
a relocated dylib, static HTTP delivery and failure rollback. It requires a matching
wasm-bindgen CLI on PATH and the wasm32 Rust target. Router URL tests run without a
browser/GPU. These checks do not establish browser/GPU execution, Developer ID
access, notarization or Linux distribution compatibility.
