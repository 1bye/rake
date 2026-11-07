# Standalone bundle configuration

`Bundle.toml` is an authoritative input to the existing `rake-dist-core::Config`
and distribution planner. It does not merge with Cargo package metadata. This
implements RFC #25 step 1; build-script handoff, Cargo sidecars/helper apps, SDK
bindings, release/doctor parity and standalone Web dispatch are subsequent work.

```toml
schema_version = 1
name = "My App"
identifier = "com.example.my-app"

[program]
type = "cargo"
# manifest = "Cargo.toml"
# package = "my-app"  # Required if workspace selection is ambiguous.
# bin = "my-app"      # Required if binary selection is ambiguous.
# features = ["feature-one"]
```

A plain Cargo application needs no Rake runtime, UI dependency or build hook.
Its version comes from locked/offline Cargo metadata. `program.version` and
`program.path` are invalid in Cargo mode. The planner lists available package or
binary names when selection is missing or invalid. Invocation `--features`
augments declared features; `--debug` selects the Cargo debug profile.

For an already built loose executable:

```toml
schema_version = 1
name = "Native Viewer"
identifier = "com.example.native-viewer"

[program]
type = "prebuilt"
path = "build/{target}/native-viewer{exe}"
version = "1.0.0"
target = "aarch64-apple-darwin"
```

Prebuilt inputs require a semantic version and a target in the file or via
`--target-triple`. Targets must agree when both are supplied. `{exe}` is `.exe`
on Windows and empty elsewhere. Planning reads the native executable's actual
architecture and file format. Scripts, directories, objects/shared libraries, wrong architectures and
non-executable Unix-target files fail. Prebuilt mode invokes neither Cargo nor rustc, even
for host detection. Cargo fields (`manifest`, `package`, `bin`, `features`),
`--features`, `--debug`, universal builds and Web configuration are invalid for
prebuilt programs. An explicit Windows manifest may be applied; prebuilt inputs
do not inherit the default Windows manifest.

## Selection and paths

1. `--config FILE` selects that standalone file.
2. Otherwise discover exactly `./Bundle.toml` in the invocation directory.
3. Otherwise use the existing Cargo metadata adapter. An explicit
   `--manifest-path` selects that adapter and suppresses discovery.

`--config` conflicts with `--manifest-path`, `--package` and `--bin`. With a
discovered file, `--package`/`--bin` also conflict: put them in `[program]`.
There is no ancestor discovery. A selected invalid or missing explicit file fails;
it never falls back to metadata. Plans report the selected source and any
shadowed `package.metadata.rake.bundle` without parsing/merging those settings.
Prebuilt shadow reporting reads only a root-level Cargo.toml, without tools.

The canonical input root defaults to the config directory. An explicit ancestor
root is useful when keeping configuration under `packaging/`:

```toml
# packaging/Bundle.toml
schema_version = 1
root = ".."
name = "My App"
identifier = "com.example.my-app"

[program]
type = "cargo"
```

All declared payload paths and the default `dist/` output are relative to that
root. The selected config must remain inside it. Input traversal, absolute input
paths, escaping symlinks and escaping symlinks inside payload trees fail. Tool,
signing-store and publisher references retain their existing external-path rules.
Explicit `--output` is relative to the invocation directory. Output cannot overlap
inputs, the config, manifest or lock, and cannot itself be a symlink.

The standalone component lock is `<config-stem>.lock` beside the selected file:
`Bundle.toml` uses `Bundle.lock`; `packaging/Custom.toml` uses
`packaging/Custom.lock`. Legacy metadata keeps `rake-dist.lock`. Programmatic
sources supply a canonical root and explicit absolute lock path. Plans create no
output, cache or lock directories and do not update either lockfile.

## Planning and schema

```sh
rake bundle --plan
rake bundle --config packaging/Bundle.toml --plan --json
rake bundle --schema > bundle-file.schema.json
rake bundle --legacy-schema > legacy-config.schema.json
```

Standalone text plans print source, root, lock, output, target, resolved program (including Cargo profile and features),
shadowed metadata and effective configuration. JSON includes `source`, `program`,
`target`, `output`, `shadowed_metadata`, `plan`, and `native_validation`.
Native validation remains pending: planning does no builds, native packaging,
signing, downloads, secret-store access or publication. Cargo metadata inspection
uses `--locked --offline --no-deps`. Prebuilt planning works with an empty PATH.
Legacy metadata planning preserves its existing `plan`/`native_validation` shape
and adds source provenance.

`bundle_file_schema()` describes the standalone document. Its distribution
properties and nested definitions come directly from `configuration_schema()`,
which retains its legacy `Config` meaning. The only envelope keys are `root` and
`program`; schema version 1, name, identifier and program are required. All current
Config fields are retained: desktop/platform settings, dependencies/components,
resources/path-based sidecars/runtimes, overrides, updates, release, custom formats,
component recipes and Web settings. Unknown keys fail with file context, the
nested field path and a closest-key suggestion. Tagged program forms reject
mixed Cargo/prebuilt keys. Map entries such as privacy names, locales and recipe
options remain intentionally open; their existing value validation still applies.

Migration fixtures preserve all ten current examples' configuration. Legacy
examples explicitly opt into schema 1 and spell out any previously implicit
identity; all other Config values remain identical. Full-model round-trip tests
also cover platform settings, target/format overrides, updates, release,
extensions and Web data.

## Worker and integration limits

`BundleSource` owns the existing `Config`, canonical root, explicit lock and
program. `SourceBundleRequest` adds invocation overrides. `plan_source` resolves
that request, and `bundle_source` sends its normalized source snapshot through the
existing owned worker. The worker revalidates inputs and Cargo selection without
rereading a live Bundle.toml. Existing `BundleRequest`, `bundle`, `execute_worker`
and metadata planning APIs retain their meaning.

The packager host is the executable implementing this worker protocol, not the
application being packaged. See [the independent host](../crates/rake-dist/examples/packager-host.rs),
[plain Cargo fixture](../crates/rake-dist/tests/fixtures/plain-cargo), and
[prebuilt fixture](../crates/rake-dist/tests/fixtures/prebuilt).
`dispatch_worker` returns typed errors, never exits or prints, and must be handled
before starting an embedding host's runtime.

Standalone updates, Velopack and Flatpak fail during planning with specific
unsupported-integration errors. They require the later build-context or source
integration. Cargo packages using the `rake-build` build dependency can be planned,
but execution fails before building until the step-2 handoff is implemented;
otherwise build.rs could embed conflicting metadata. Ordinary resources and native
metadata remain packaging inputs; applications own their runtime lookup behavior.
Standalone Web bundling fails explicitly; `Config.web` remains representable and
validated by the existing model, and explicit legacy Web commands remain available.
Cargo sidecar bins, macOS helper declarations and `program.build_env` are rejected
until their respective RFC steps. No standalone release/promotion claim is made.
