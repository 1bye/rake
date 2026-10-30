# Distribution adoption progress

P0 is in progress. Rust 1.95.0 on macOS arm64 compiled both isolated adapters
and passed the common unsigned app probe for tauri-bundler 2.10.4 and
cargo-packager 0.11.8. Both packages contained the resource and executable helper;
the relocated fixture and helper ran from an unrelated working directory. Cargo
input binaries retained their original hashes. Both adapters pass Clippy with
warnings denied.

The fixture is a separate Cargo workspace so no packaging dependency enters the
Rake runtime or main workspace lockfile. Its committed lockfile fixes the test
resolution. Select exactly one feature when building it.

Run the probes with:

```sh
python3 scripts/test-distribution-adoption.py --output /tmp/adoption-results.json
```

The cost-bounded Actions workflow uses three native jobs, with both engines
sharing each host's dependency cache. It triggers only on changes to the adoption
fixture/workflow on codex/distribution, or explicit workflow_dispatch. Later
implementation commits outside those paths do not rerun it.

Windows probes create an unsigned NSIS artifact. Linux probes extract a DEB and
run its executable/helper. Neither is an installed-system upgrade test. The
probes report pending checks explicitly instead of treating them as passed.

The full P0 gate is still open: native Windows/Linux results, custom metadata,
framework/dylib relocation, signing, tool acquisition, cancellation and updater
save/restart handoff need evidence. Tauri remains the proposed engine. No
production engine dependency or claim of multi-platform distribution support
has been added.

## First native matrix

Run https://github.com/1bye/rake/actions/runs/41057984993 passed on macOS and
Ubuntu. Windows compiled both engines; the cargo-packager NSIS invocation failed
because the adapter supplied mixed path separators. Normalize PathBuf components
before passing them to the engine. Follow-up runs are manual and can select only
the failing OS. No full matrix rerun is needed.

Additional local macOS probes pass custom plist, relocated linked framework,
internal framework symlinks, and external inside-out ad-hoc signing followed by
strict signature verification for both engines. This does not establish
Developer ID signing, notarization, or the SDK restart handoff.
