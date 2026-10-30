# Native distribution acceptance

Portable tests validate contracts; the following tests exercise installed programs.
The manual Distribution adoption workflow accepts one platform to limit runner
cost. Commits use `[skip ci]` and the workflow is dispatched only at checkpoints.

## macOS Sparkle

Use `python3 scripts/prepare-distribution-sparkle.py --output /tmp/sparkle-test`.
This creates an isolated routed GPUI application and a fresh disposable Ed25519
key. Keep all private test files outside the feed and application resource tree.
The test-only `native-tests` feature permits an explicit loopback HTTP feed; normal
applications require HTTPS and have no override.

Build version 1.0.0 with `rake bundle --manifest-path /tmp/sparkle-test/source/Cargo.toml
--debug --format app --output /tmp/sparkle-test/v1 --json`. Copy the resulting app
to a separate installation directory. Change the fixture version to 1.0.1, update
its Cargo lockfile, and bundle `--format zip` into a separate v2 directory. Put only
the new ZIP in a feed directory. Use the pinned Sparkle SDK's `generate_appcast`
with `--ed-key-file`, `--maximum-deltas 0` and a loopback download URL prefix.
Serve that directory on the selected port. Never serve the fixture root.

Launch the installed v1 through LaunchServices, then:

1. Check for updates, install, and choose Install and Relaunch. Confirm that Rake
   displays the dirty-document Save/Discard/Cancel decision.
2. Cancel. Confirm that the original app version and unsaved document remain.
3. Retry, including the SDK's cached installation path. Choose Save. Confirm
   automatic relaunch at 1.0.1, the saved document and the persistent data marker.
4. Alter the feed version without signing again. Confirm Sparkle rejects the
   improperly signed feed. Restore its signed bytes and retry.
5. Exercise an OS quit and confirm the native termination request enters the
   same guard. The `native-quit` example separately sends actual AppKit terminate
   requests, tests deferred cancellation/approval and restores the prior delegate.

A local run on 2026-10-30 completed steps 1–4 using Sparkle 2.10.1, including the
cached installation path. Version 1.0.1 wrote its startup marker before any UI
reattachment, proving SDK relaunch. The native termination protocol probe passed.
This is development ad-hoc acceptance; Developer ID/notarization has separate
recorded acceptance. Production private keys were not used for update testing.

## Linux and Windows

`scripts/test-distribution-linux.py --cli target/debug/rake --install` exercises
native Debian and Fedora installation, unrelated working-directory launch,
upgrade and uninstall. Fedora uses an isolated container.

`scripts/test-distribution-velopack.py` exercises the pinned native vpk channel
and uses only a disposable runner certificate. The Windows SDK tests also inspect
PE metadata and create/install/upgrade/remove MSIX packages. Missing credentials
or native prerequisites fail the explicit acceptance suite; they do not count as
passing a gate.
