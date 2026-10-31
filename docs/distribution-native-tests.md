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


## Focused native gates and evidence

- `scripts/test-distribution-windows-installers.py`: signed NSIS/MSI installation,
  private CRT, protocol/file registrations, N+1 upgrade, unrelated-CWD resources,
  uninstall and an actual installed minimal GPUI frame. `--only` selects one gate.
- `scripts/test-distribution-velopack-update.py`: generated signed HTTPS feeds,
  untrusted/tampered metadata, cancellation/decline, cached package corruption and
  native SDK automatic N+1 relaunch with persistent data.
- `scripts/test-distribution-appinstaller.py`: generated HTTPS App Installer
  enrollment, native channel association, managed N+1 installation and persistent
  data. Its disposable CA is enrolled only on an isolated Actions runner.
- `scripts/test-distribution-appimage.py`: publish N and N+1 separately, remove old
  build directories, then authenticate installed N from retained signed history.
  Actual image execution covers signature/truncation/insufficient-space rejection,
  cancellation, competing updater locks, read-only installations, atomic N+1
  replacement and automatic relaunch/data preservation.
- `scripts/test-distribution-linux.py --repositories`: signed APT/RPM repositories,
  fresh native installs and package-manager update discovery. Fedora uses `fedora:43`.
- `scripts/test-distribution-linux-formats.py`: Arch makepkg/pacman and Flatpak.
  The GUI is built from source within its pinned SDK; frame callbacks verify its
  installed resource. Wayland uses a nested Weston compositor with an input seat.
  Signed Flatpak repositories use native worker preparation, verified handoff,
  fresh N enrollment and native N+1 upgrade. `--reuse-flatpak` checks hashes of an
  explicitly retained GUI artifact while rebuilding the small repository fixture.

Cost-bounded workflow modes select only outstanding gates. `flatpak` with an
explicit `fixture_run` reuses `native-flatpak`; the nine-minute GUI source build
is not repeated for repository or compositor diagnostics. `appinstaller` skips
already-passed Windows updater, installer and UI suites. The initial broad
comparison is not rerun for phase follow-ups. Cancel acknowledgements are not
terminal job states; wait for completion before dispatching a replacement.

Recorded checkpoints:

| Run | Passing native gates |
| --- | --- |
| [41104892784](https://github.com/1bye/rake/actions/runs/41104892784) | Signed MSIX install/upgrade and real GPUI packaging |
| [41108529192](https://github.com/1bye/rake/actions/runs/41108529192) | Velopack installer N/N+1 lifecycle |
| [41111628107](https://github.com/1bye/rake/actions/runs/41111628107) | APT/RPM/Arch repositories and signed native AppImage update |
| [41130631879](https://github.com/1bye/rake/actions/runs/41130631879) | Velopack HTTPS SDK update/relaunch, native DLL isolation, localized MSIX/PRI and NSIS registrations |
| [41132236560](https://github.com/1bye/rake/actions/runs/41132236560) | Signed MSI lifecycle/registrations and installed minimal Windows UI |
| [41130639063](https://github.com/1bye/rake/actions/runs/41130639063) | Expanded AppImage failures/history, Linux legacy compatibility and source-built Flatpak X11 frame |
| [41133766124](https://github.com/1bye/rake/actions/runs/41133766124) | Flatpak X11/Wayland frames, signed repository/ref enrollment, verified native handoff and N+1 upgrade/data preservation |
| [41136419908](https://github.com/1bye/rake/actions/runs/41136419908) | HTTPS App Installer enrollment/association, native managed activation, N+1 upgrade/data preservation and Windows helper tree lifetime |

Some earlier checkpoint jobs failed at a later independent gate; the table names
only steps that passed. The final Linux and Windows runs above passed completely.
The initial native matrix and required P0–P9 gates are complete. MSIX launch uses
Windows package activation; the fixture never changes protected installation ACLs.
The release CI archive transport also passed a real Developer ID framework
roundtrip with executable modes, relative symlinks, bytes and signature intact.

The minimal macOS example was also bundled ad hoc, run from `/`, rendered its
native frame and read its installed resource on 2026-10-30. Running it requires
WindowServer access outside the filesystem sandbox. The same `native-acceptance`
feature is used on all three initial platforms. Production trust and Sparkle
acceptance above remain separate from this development UI fixture.

## Shared updates controller

The `updates-controller` mode of `distribution-adoption.yml` runs only Linux and
Windows controller tests, native host compilation, and the existing signed HTTPS
AppImage/Velopack N→N+1 fixtures. Their check, download, decline and restart paths
use `UpdateController`; provider-specific corruption/cancellation cases remain in
the same fixtures. No engine comparison or Flatpak/installer matrix is required.

On macOS, `scripts/test-distribution-sparkle-state.py --cli target/debug/rake
--appcast /path/to/signed/appcast.xml --public-key /path/to/sparkle.pub` builds
isolated bundled probes against an existing Sparkle 2.10.1 signed feed advertising
1.0.1. It checks availability from 1.0.0, UpToDate from 1.0.1, cancellation and
signature rejection, plus zero native windows and no payload download. Run outside
the sandbox with an active graphical login. It also verifies checking without a
quit guard, rejecting installation without one, and attaching a guard afterward.
This probe does not install an update
or alter signing profiles. The existing native Sparkle installation gate remains
responsible for full SDK installation and guarded termination.
