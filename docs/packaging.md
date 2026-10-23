# Packaging and distribution

`rake bundle` uses the same package selection, Cargo build and Wasm binding generator
as `rake build`. It needs no running app, dev supervisor or runtime connection.

```sh
cargo build -p rake-cli
target/debug/rake bundle --target desktop --package rake-routing --json
target/debug/rake bundle --target web --package rake-routing --base-path /notes/ --routing path --json
target/debug/rake bundle --target web --package rake-routing --routing hash
```

Bundles default to the **release** profile. Use `--debug` explicitly for local
packaging checks. Existing `--manifest-path`, `--package`, `--bin`, `--features`,
`--web-dir` and `--output` options work as for builds. Defaults write
`dist/PACKAGE/desktop-bundle` or `dist/PACKAGE/web-bundle`. Build outputs remain
separate at `dist/PACKAGE/desktop|web`. Python 3.11+ is required; all packaging
helpers are bundled in the CLI. Web also requires the wasm32 target and a
`wasm-bindgen` CLI exactly matching Cargo.lock.

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
icon = "icons/notes.png" # Optional; .png or .icns on macOS
build_number = "12"     # Optional; defaults to the numeric Cargo version
minimum_system_version = "13.0" # Optional macOS deployment declaration

[package.metadata.rake.bundle.web]
base_path = "/notes/"
routing = "path"
```

The display name defaults to the Cargo package name. The identifier defaults to
`dev.rake.PACKAGE`, with underscores replaced by hyphens. Set it to the application
ID passed to `App::new` for consistent OS/runtime identity. Cargo's full version
is retained in the manifest/archive; the numeric part supplies the macOS short
version. Build numbers use one to three numeric components. The minimum OS value
declares compatibility; it does not change Rust/linker deployment targets or prove
that dependencies support that OS.

Asset/icon paths are explicit, package-relative inputs. Files outside the package,
symlinks, special files and output-name collisions fail. The asset tree is copied
without following links; static resource permissions are normalized. Only declared
asset trees and the Web host tree are copied, not the project or dev-session data.
This stage packages resources; applications choose how to decode or register them
with GPUI. Runtime paths are:

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

On macOS the artifact is a versioned `.app` with an executable, Info.plist,
resources and optional icon. PNG icons are converted to ICNS during bundling.
Plain non-system dylib dependencies are resolved from loader/executable paths and
LC_RPATH entries, copied recursively into `Contents/Frameworks`, rewritten and
signed before the outer app. Filename collisions and unresolved libraries fail.
External framework bundles/plugins need a platform packager and are rejected;
system frameworks and `/usr/lib` dependencies remain system-provided.

The default is an **ad-hoc signature**, verified with `codesign --verify --deep
--strict`. This supplies local bundle integrity, not Developer ID/notarization or
Gatekeeper distribution approval. For an explicitly selected installed identity:

```sh
target/debug/rake bundle --target desktop --package notes \
  --sign-identity "Developer ID Application: Example (TEAMID)" \
  --entitlements entitlements.plist
```

Identity signing uses a secure timestamp and Hardened Runtime for the main app;
entitlements are optional. Signing failure prevents publication. The ZIP is made
with `ditto --keepParent` after signature verification, following Apple's
[distribution signing](https://developer.apple.com/documentation/xcode/creating-distribution-signed-code-for-the-mac/)
and [packaging guidance](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution).
Notarization, stapling, DMG/installer creation and App Store submission are separate
steps; no credentials are inferred or upload performed. `notarized` remains false.

Linux produces a directory archive with `bin/` and `share/` resources. It relies
on compatible system libraries and is not an AppImage, distro package or installer.
The layout is checked on macOS; Linux native execution/distribution still needs a
Linux host validation. The CLI's native process/transport layer currently requires
Unix; Windows packaging is unsupported. macOS archives report the actual Mach-O
architecture; builds use Cargo's selected host target, with no universal merge.

## Web deployment

The `site/` directory and ZIP contain the Wasm module, exact matching JS bindings,
host/loader and declared assets. The ZIP has site contents at its root. The host
contains `rake-web-config` metadata: the runner applies it before initial route
decoding and uses it for subsequent History API writes. Both path and hash routing
preserve the prefix, including Back/Forward and clearing the last tab. CLI flags
override bundle Web metadata. Development builds retain their normal root host;
`rake bundle` applies deployment settings.

`WebConfig` owns its prefix and is cloneable. Applications can also configure it
directly with `WebConfig::new().base_path("/notes/")?`; `uri_for` and
`destination_from_url` share the browser adapter's typed deployment codec.

Base paths begin/end with `/`, use ASCII letters/digits, `-`, `_`, `.`, `~`, and
nonempty segments, and reject dot segments or encoded/query/fragment syntax.
Rake adds `<base>`, rebases `/pkg/` and `/assets/` references in custom HTML/JS/CSS,
and leaves generated bindings' module-relative Wasm resolution intact. Custom
root-relative application/API URLs remain author-owned. Remove a custom `<base>`
element before bundling. `pkg/`, `rake-web-config` and `rake-deployment.json` are
reserved. Inputs containing development bootstrap/bridge metadata are rejected.

Upload **site contents** below the configured URL prefix. `rake-deployment.json`
records routing/base/assets/MIME/cache requirements, and `nginx.conf.example`
supplies path/hash host rules. Path mode needs app-shell fallback for destination
URLs; missing Wasm/asset files must return 404. Hash mode needs no destination-path
rewrite. Serve `.wasm` as `application/wasm` and JS with its JavaScript MIME type.
Serve index/loader/bindings/Wasm with revalidation: these filenames are not content
hashed, so immutable caching would mix releases. Deploy the complete directory as
one version to avoid mixing old bindings and new Wasm.

No broker, dev bootstrap, inspector assets, token, reload checkpoint or connection
metadata is added to production output. A normal static server is sufficient.
Rake creates artifacts and host guidance; it does not upload to a hosting account.

## Verification

```sh
python3 scripts/test-bundle.py
cargo test -p rake --no-default-features --test web_deployment
```

The packaging suite uses real tiny Rust/Wasm apps, platform codesign/archive tools,
a relocated dylib, static HTTP delivery and failure rollback. It requires a matching
wasm-bindgen CLI on PATH and the wasm32 Rust target. Router URL tests run without a
browser/GPU. These checks do not establish browser/GPU execution, Developer ID
access, notarization or Linux distribution compatibility.
