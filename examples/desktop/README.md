# Small desktop application

A compact native application showing the complete path from a `rake::view!` UI
to distribution. Run commands below from the repository root.

```text
examples/desktop/
├── Cargo.toml              # Dependencies, icon, packages, updates and publishing
├── build.rs                # rake-build preparation
├── src/
│   ├── main.rs             # App, shared AppInfo service and theme
│   ├── shell.rs            # Persistent Render layout; no Page implementation
│   └── pages/
│       ├── mod.rs          # Typed navigation addresses
│       ├── home.rs         # Home: Page + Render, resources and dirty state
│       └── updates.rs      # Updates: Page + Render, controller and guarded restart
├── assets/
│   ├── icon.svg
│   └── welcome.txt
└── packaging/
    ├── Info.plist          # Additional macOS metadata
    └── app.manifest        # Windows metadata, preserving GPUI requirements
```

## Run and navigate

```sh
cargo run -p rake-desktop-example
```

Home and Updates navigate in the current tab. Back/Forward reuse retained views;
the local edit count survives. Make an unsaved change on Home, open Updates, then
Quit: the routed host offers Discard/Cancel before disposing that dirty history.
“Mark clean” demonstrates clearing dirty state; it does not persist a document.
Update restart uses the same guard path. The host still presents guard decisions
when `workspace_chrome(false)` hides its default tab/quit controls.
Navigation loading, Cancel, and Retry also remain available outside the shell.
The starter limits each tab to 64 retained entries; reaching the limit blocks
the next push instead of discarding unsaved state. Low-level routers default to
unlimited history.

`#[rake::app]` runs installer startup and passes build-script distribution policy
to `App`. The app registers `AppInfo` once. An owned `PageContext` is passed to
each registered `Page::mount`;
`page.services().get::<AppInfo>()` resolves that same shared value.
Each screen retains its own `PageContext`, so navigation and services remain
accessible from click handlers after mounting.

The app registers its page types directly:

```rust
App::new("dev.rake.starter")
    .theme(colors)
    .routes(rake::routes! {
        Destination::Home => loaded(Home),
        Destination::Updates => Updates,
    })
    .try_layout(Shell::mount)
    .initial(Destination::Home)
```

`Home` implements `LoadedPage<Destination>` and `Updates` implements
`Page<Destination>`, with ordinary `Render` implementations in their own files.
The macro checks every destination and calls the selected page's constructor;
there is no application-owned dispatcher. Mount returns an `Entity<Self>` and
runs only for a new or reconstructed history entry. Theme installation runs once
at app startup before any page mounts. `Shell` is one persistent `Render` entity
per window. Its `LayoutContext` supplies the shared services, navigation and route
outlet. Pages render their bodies; they do not construct the shell. The shell's
navigation-click count survives page changes independently of retained page edits.
`try_layout` reports initialization failures without panicking.

Home's route loader reads the bundled welcome text on a background worker and
passes an owned `String` into mount. A failed load leaves the old page intact and
shows the host's Retry control. It completes before a new physical window opens.
Back/Forward reuse the retained view and data; selecting Home again creates a
fresh entry and loads again. Loaders may prepare files, services or domain models;
they are not limited to HTTP. Blocking work must explicitly use a worker.

Updates resolves the automatically registered `UpdateController` during mount.
Its `page.while_visible` observer awaits `changed(revision)` to redraw, pauses
while hidden, and publishes a fresh snapshot when shown again. Rake owns the provider driver and
blocking work; views do not create worker threads. Check, available version,
download progress, cancellation, failures and guarded restart use that controller.
No update check starts automatically. An unbundled `cargo run` has no installed
controller and shows that updates are unavailable. This is a desktop example;
[the view](../view) and [routing](../routing) examples also cover WebAssembly.

Optional development inspection:

```sh
RAKE_DEV_CONTROL=1 cargo run -p rake-desktop-example --features devtools
```

## Resources and packaging

`welcome.txt` is read through `rake::resources!()` in an installed app. Only an
unbundled development run uses the embedded development copy. Corrupt or missing
installed resources report a mount error. One SVG produces native icon formats.
The macOS plist is merged with generated identity; the Windows manifest preserves
GPUI's DPI, compatibility, long-path and heap settings. Linux desktop metadata is
generated from the common identity/category.

```sh
cargo run -p rake-cli -- doctor --distribution --package rake-desktop-example --json
cargo run -p rake-cli -- bundle --package rake-desktop-example --plan --json
cargo run -p rake-cli -- bundle --package rake-desktop-example --debug --output examples/desktop/dist --json
# A quick macOS app bundle without building a DMG:
cargo run -p rake-cli -- bundle --package rake-desktop-example --debug --format app --output examples/desktop/dist-app --json
```

Run native package builds on their own OS. Defaults here are ZIP/DMG on macOS,
Velopack on Windows, and AppImage/DEB/RPM/Arch on Linux. Use explicit `--format`
while developing one format. `bundle --plan --platform windows` or `--platform linux`
can inspect another platform without its native SDKs. Packagers/SDKs are build
machine prerequisites; users receive the resulting installed payload.

## Enable real updates and signing

The checked-in example runs without signing credentials: `updates.enabled` is
false, and there are no fabricated verification keys or live publisher endpoints.
To configure your own app:

1. Replace the identifier, name and both `downloads.example.com` URLs in Cargo.toml.
   `updates.base_url` and `release.publisher.public_url` must point at the same
   HTTPS publication root. Adjust the required target matrix for your architectures.
2. Generate AppImage and Velopack keys outside bundled inputs. These commands
   create protected private files and public files suitable for committing:

   ```sh
   cargo run -p rake-cli -- update-key generate --private-key /secure/starter-appimage.key --public-key examples/desktop/packaging/appimage.pub
   cargo run -p rake-cli -- update-key generate --private-key /secure/starter-velopack.key --public-key examples/desktop/packaging/velopack.pub
   ```

   `/secure` stands for an existing private directory you own. The example's
   `release.signing_keys` uses secret-manager environment references. Supply the
   respective base64 seeds as `RAKE_STARTER_APPIMAGE_SEED` and
   `RAKE_STARTER_VELOPACK_SEED` on the publishing machine, or change each reference
   to `kind = "file", path = "/secure/..."`. Native macOS publication can instead
   use `kind = "keychain", service = "...", account = "..."`.
3. For Sparkle, use the pinned 2.10.1 SDK's `bin/generate_keys --account rake-starter`
   to enroll its Keychain signing key. `bin/generate_keys --account rake-starter -p`
   prints only the existing public key; save it to `packaging/sparkle.pub` inside
   this example. Uncomment the `release.sparkle_tool`/`sparkle_account` settings
   and supply the SDK's actual `generate_appcast` path.
4. Uncomment `updates.public_keys` and set `updates.enabled = true`. Provide keys
   for the platforms you distribute. `rake-build` reads every declared key file,
   so remove entries for platforms not configured yet. If dropping a platform,
   also remove its entry from `release.required`.
5. Set macOS Developer ID/notarytool profile names and the Windows certificate
   thumbprint in the indicated fields. These reference enrolled local stores;
   update keys, code-signing identities and notarization profiles are distinct.

Bundle builds automatically select the Rake provider feature for their format.
On macOS they also prepare the pinned Sparkle framework. Hand-written Cargo SDK
development builds need `--features rake/updates-sparkle` and
`RAKE_SPARKLE_FRAMEWORK=/path/to/Sparkle.framework`; use `rake bundle` for the
installed app workflow. Sparkle availability checks are silent; Download opens
its native review UI. AppImage/Velopack stage through the shared controller.
DEB/RPM/Arch report package-manager ownership instead of replacing themselves.

## Publish

Inspect the configuration without loading credentials or publishing:

```sh
cargo run -p rake-cli -- release --package rake-desktop-example --plan --json
```

After configuring your own destinations, keys and local signing stores, build
trusted artifacts on each OS. `--trusted` verifies native signing/notarization:

```sh
# Run on each corresponding native worker, using a fresh output directory:
cargo run -p rake-cli -- bundle --package rake-desktop-example --trusted --output /path/to/native-output --json
```

Transfer the artifact directories to the publishing machine. One promotion checks
the whole required matrix and exports native feeds plus immutable objects:

```sh
cargo run -p rake-cli -- release --package rake-desktop-example \
  --from /path/to/mac-output --from /path/to/windows-output --from /path/to/linux-output --json
```

The configured directory publisher writes `examples/desktop/release-site` for
your static HTTPS server. Hosting that directory remains your deployment step.
For one platform, reduce `release.required` before building; plain `release` can
build and publish the current host when that is the entire required matrix.
For version N+1, bump the Cargo package version, rebuild, and promote to the same
channel; preserve authenticated history in the publication directory.

For direct GitHub Releases, replace the publisher table with:

```toml
[package.metadata.rake.bundle.release.publisher]
kind = "github"
repository = "your-org/your-app"
tag = "v0.1.0"
```

Set `updates.base_url` to
`https://github.com/your-org/your-app/releases/latest/download` and create that
tag yourself before publishing. [Release documentation](../../docs/distribution-release.md)
covers HTTPS promotion services, signed Linux repositories, Flatpak and MSIX
enrollment. The small default config exports Linux packages, not a configured
APT/RPM/pacman repository; add the documented GPG/repository settings for those.

## Optional bundled FFmpeg

Prepare `vendor/ffmpeg/<rust-target>/` with `bin/ffmpeg`, `bin/ffprobe`, `LICENSE`
and, for dynamically linked builds, `lib/`. Windows executable names end in `.exe`.
Uncomment the `dependencies.ffmpeg` table in Cargo.toml. Rake packages the tree,
checks native dependencies and licenses, and exposes its helpers:

```rust
let resources = rake::resources!()?;
let ffmpeg = resources.helper("ffmpeg")?;
// Dispatch any blocking subprocess work on an owned background worker.
let command = std::process::Command::new(ffmpeg);
```

For a cross-platform dependency plan, include the exact target, for example
`bundle --package rake-desktop-example --plan --platform windows
--target-triple x86_64-pc-windows-msvc --json`. Native bundle execution selects its
host triple automatically.

No FFmpeg binaries are checked into this sample. Keep a real distribution's
public keys and generated `rake-dist.lock` under version control after configuring
its inputs. GStreamer/library groups use the same dependency configuration;
see [distribution](../../docs/distribution.md#native-settings-and-dependencies).
For richer managed tasks, HTTP/data and typed application commands, see
[routing](../routing) and [notes](../notes); they use the same app/services boundary.
