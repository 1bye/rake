# Releasing native applications

`rake bundle` creates local artifacts. `rake release --plan --json` reports public
configuration without loading signing credentials or publishing. An explicit
`rake release` builds with release trust and locked native components, then
publishes. Use repeated `--from <bundle-directory>` to promote a complete native
runner matrix. Existing signed AppImage history is retained by the publisher.

```toml
[package.metadata.rake.bundle.release.publisher]
kind = "directory"
path = "release-site"
public_url = "https://downloads.example.com/my-app"

[package.metadata.rake.bundle.release.required]
aarch64-apple-darwin = ["zip", "dmg"]
x86_64-pc-windows-msvc = ["msix", "velopack"]
x86_64-unknown-linux-gnu = ["deb", "rpm", "appimage"]
```

Serve that directory through a static HTTPS server. Immutable content is stored
under `objects/`; each native feed tree lives under `<channel>/<target>/`.
Directory export owns its root and refuses an existing unrelated directory. On
Unix, an atomic symlink replacement switches the whole channel tree while keeping
previous generations and objects. Windows export uses the owned directory
replacement transaction. Use a Unix static host for atomic reader visibility.
No retention deletes happen automatically; keep prior versions needed for update
ownership verification and compatible data recovery.

The release verifies every input hash, requires all configured targets at the
selected version, and refuses older/replayed directory promotions. Failed builds,
missing targets, signing errors and failed object readback prevent promotion.
Native macOS releases require recorded Developer ID, notarization and stapling;
Windows installer executables require recorded Authenticode and RFC3161 timestamp
verification.

## Update metadata and local signing profiles

Enable updates in the bundle configuration and provide public-key files. Private
keys remain in native signing stores or explicitly configured signing commands.
Sparkle uses the pinned SDK's `generate_appcast` with its existing Keychain account:

```toml
[package.metadata.rake.bundle.release]
sparkle_tool = "/path/to/Sparkle-2.10.1/bin/generate_appcast"
sparkle_account = "ed25519"
feed_validity_seconds = 604800

[package.metadata.rake.bundle.release.signers]
appimage = ["tools/sign-appimage-feed"]
velopack = ["tools/sign-velopack-feed"]
```

A signer receives the exact bounded feed bytes on stdin and returns exactly 64 raw
Ed25519 signature bytes on stdout. It may use an existing local key profile; logs
belong on stderr. Rake verifies its signature against the configured public key
before uploading. Sparkle's native signed appcast is also verified against its
public key; the SDK obtains native build counters and enclosure metadata from the
actual archives. Velopack keeps its native release JSON plus a context/expiry-bound
signature sidecar. MSIX App Installer identity comes from the built artifact record.

Publish N once, then supply N+1 outputs on the next release. Directory, GitHub and
HTTPS publishers merge the previous authenticated AppImage history under the
promotion lease/precondition, including history whose old feed has expired. Old
build directories are not required. Tampered history and version forks prevent
promotion. Retain the immutable published objects/releases.
The AppImage adapter authenticates the installed N image against this history;
removing that record prevents an old installation from authorizing replacement.

## GitHub and HTTPS

For GitHub Releases, configure `kind = "github"`, `repository = "owner/repo"`, and
`tag = "v1.2.3"`. The tag must already exist. Rake uploads to a draft, downloads and
hashes every asset, then publishes it as latest. Drafts remain available when an
upload fails. Use the serialized release workflow for all publishers of a repo.
Update roots use `https://github.com/owner/repo/releases/latest/download`; Rake
maps native feed paths into GitHub's flat asset namespace. Keep channel series in
separate repositories when using this latest-release endpoint. Linux repository
trees require a directory or HTTPS publisher.

The HTTPS publisher targets an explicitly enrolled object/promotion service. It
requires a public static URL, an HTTPS API root and `token_env`, naming a scoped
credential environment variable. It uploads immutable objects with If-None-Match,
reads them back, then conditionally submits the entire channel state using the
previous ETag. The service must implement atomic channel-tree promotion described
in `Publisher::Https`; an ordinary HTTP file server is served from directory
export instead. Signing profiles and tokens never enter artifact manifests.

## Linux repositories

```toml
[package.metadata.rake.bundle.release]
gpg_key = "0123456789ABCDEF0123456789ABCDEF01234567"

[package.metadata.rake.bundle.release.repositories]
x86_64-unknown-linux-gnu = ["deb", "rpm", "arch", "flatpak"]
```

The platform owner uses dpkg-scanpackages/apt-ftparchive/GPG, rpmsign/createrepo_c,
repo-add/GPG and flatpak/ostree. It exports public enrollment keys, signs native
metadata, and builds repositories in isolated staging before promotion. Install
those native tools on the Linux publication worker and enroll its signing profile
once. The Flatpak repository exports `.flatpakrepo` and `.flatpakref` files.
Application signing and repository trust enrollment are distinct setup steps.

For a combined native runner matrix, prepare repositories on the Linux worker:

```sh
rake release --from release-output --prepare-repositories release-repositories --json
# Transfer the bundle directories and repository export through trusted CI storage.
rake release --from mac-output --from windows-output --from linux-output \
  --repositories release-repositories --json
```

Preparation never publishes and only prepares repositories for supplied targets.
Final promotion still checks every required target/format, exact source package
hashes, export identity/version/channel/destination and every repository file hash.
The final publisher can run on macOS for Sparkle tooling while consuming the
already signed Linux repositories. Native workers and artifact transport are
trusted as they are for `distribution.json`; hashes detect altered payloads,
and native signatures establish installed repository trust.

`.github/workflows/distribution-release.yml` provides explicit manual native
builds, Linux repository handoff and serialized promotion. Signing jobs require
enrolled self-hosted native runners; ordinary verification remains independent of production credentials.
Publishing defaults to false. This workflow has not been dispatched as a real
release during implementation.

Provider verification keys are immutable within an update channel. Publication
records fingerprints of the actual decoded keys, accepts equivalent public-key
encodings, and refuses silent replacement for Sparkle, Velopack and AppImage.
To rotate a trust root, publish a new channel and enroll it through a normally
signed installer. Keep the previous channel available for its installed users;
installing that new build is the explicit trust transition. Automatic in-place
key rotation is not claimed. GitHub latest-feed channels use separate repositories,
as described above. Renewing a native code-signing certificate does not require
replacing the provider's update-verification key.
