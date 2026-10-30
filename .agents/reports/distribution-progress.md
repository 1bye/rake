# Distribution implementation progress

Branch: `codex/distribution`, in the fresh managed `distribution/rake` worktree.
The original checkout is unchanged. Every implementation phase has separate commits;
follow-up fixes keep their phase labels. P0–P9 implementation and required native gates are complete for the initial
supported profiles. Generated Windows App Installer HTTPS enrollment, native
activation, N+1 upgrade, resource reads and data preservation passed in
[41136419908](https://github.com/1bye/rake/actions/runs/41136419908).

| Phase | Result and completion evidence |
| --- | --- |
| P0 | Engine comparison/adoption complete on macOS arm64, Windows x64 MSVC and Linux x64 GNU. Tauri 2.10.4 is private to the CLI coordinator; see the adoption report. Actual minimal native frames/resources pass on all three profiles. |
| P1 | Versioned configuration/schema/planning, target/format overrides, native version mapping, ownership policy, custom recipe/backend contracts and independent verification implemented. External consumers and precise rejection cases pass. |
| P2 | Portable native CLI, isolated process groups/Windows Jobs, bounded cancellation, input confinement, immutable snapshots, output locks and transactional ownership implemented. Real bundles and resources execute after relocation; legacy Mac/Linux layouts/JSON survive through Rust. |
| P3 | PNG/SVG/default icons, installed resource descriptors, PE manifest/version integration, protected plist identity, privacy strings, desktop categories/protocols/file associations, localized macOS/Linux labels and SDK-generated MSIX PRI implemented. Native Mac metadata and Windows SDK/registrations pass. |
| P4 | Local/pinned HTTPS components, checked hashes/cache/archive bounds, component lock, licenses, owned helpers, framework/GStreamer recipes, native loader groups and private MSVC CRT implemented. Relocated FFmpeg/GStreamer processes a frame/loads plugins on macOS; real Windows DLL isolation and helper-tree cleanup pass; real ELF closure/baseline checks pass. |
| P5 | Developer ID/notary/staple, ad-hoc development trust, universal pure-Rust Mac packaging, Windows Authenticode/RFC3161, NSIS/MSI/MSIX/Velopack assembly and native installer lifecycles pass. Generated MSIX HTTPS App Installer fresh enrollment, native activation, channel association and N+1 upgrade also pass. |
| P6 | Native DEB/RPM/Arch/AppImage/Flatpak backends implemented. Install/upgrade/uninstall, signed package-manager repositories, pinned-SDK source builds and actual Flatpak X11/Wayland frames/resources pass. |
| P7 | Owned Sparkle/Velopack/AppImage providers and quit guards implemented. Actual signed N→N+1 native update/relaunch with preserved data passes for all three. Save/Cancel, cached retry, feed tampering, transfer cancellation, corrupt cache, locked/read-only images, truncation, insufficient space and killed staging have focused coverage. Managed channels cannot self-replace. |
| P8 | Directory/GitHub draft/transactional HTTPS publishers, required matrices, immutable upload/readback, signed native feeds, retained AppImage history, immutable trust roots and Linux repositories implemented. Real TLS/process protocol failures preserve channels. Signed APT/RPM/pacman/Flatpak fresh install and native N+1 upgrades pass. Linux-native repository preparation/handoff passes. Generated MSIX managed feeds also pass native fresh enrollment and N+1 upgrade. |
| P9 | Minimal `rake::view!` example, build/doctor/release workflow, configuration/docs and native Python retirement implemented. Native legacy and Web contracts pass. Final documentation and evidence match the completed initial native matrix. |

## Decisive native checkpoints

- [41057984993](https://github.com/1bye/rake/actions/runs/41057984993) and
  [41058997245](https://github.com/1bye/rake/actions/runs/41058997245): both engine adapters on all three native hosts, Windows path normalization follow-up.
- Local macOS: actual Developer ID signing, Apple notarization, stapling/ticket
  verification of a linked framework; real FFmpeg/GStreamer closure (103 native
  files), private plugins/scanner, clean PATH/CWD; native Sparkle signed update,
  dirty Cancel/cached retry/Save/automatic N+1 startup before UI reattachment,
  preserved data and tamper rejection. Actual AppKit terminate deferral/approval.
- [41104892784](https://github.com/1bye/rake/actions/runs/41104892784): signed MSIX install/upgrade and real Windows GPUI packaging.
- [41108529192](https://github.com/1bye/rake/actions/runs/41108529192): native Velopack N/N+1 installer lifecycle and early startup.
- [41111628107](https://github.com/1bye/rake/actions/runs/41111628107): signed APT/RPM/pacman repository enrollment/upgrade, Arch makepkg and actual AppImage HTTPS update/relaunch.
- [41130631879](https://github.com/1bye/rake/actions/runs/41130631879): actual Velopack signed HTTPS SDK update/relaunch with tamper/cancel/decline/cache rejection; native private DLL closure; localized MSIX/PRI SDK validation; NSIS registrations/lifecycle.
- [41130639063](https://github.com/1bye/rake/actions/runs/41130639063): retained AppImage history after deleting old builds, actual signature/truncation/disk-full failures, Linux legacy compatibility, actual pinned-SDK Flatpak source build/X11 frame/resource.
- [41132236560](https://github.com/1bye/rake/actions/runs/41132236560): signed MSI N/N+1 install/registrations/uninstall and installed minimal Windows native frame/resource.
- [41133759023](https://github.com/1bye/rake/actions/runs/41133759023): Windows root/grandchild helper lifetime passes; managed download exposed the fixture's missing HTTP range support.
- [41133766124](https://github.com/1bye/rake/actions/runs/41133766124): complete cached-GUI X11/Wayland sandbox acceptance plus signed Flatpak repository/ref fresh enrollment, native N+1 discovery/upgrade and app-data preservation, using native repository preparation/verified handoff.
- Local macOS minimal example: a fresh ad-hoc bundle launched from `/`, rendered
  its frame and read its installed resource. WindowServer access requires leaving
  the filesystem sandbox. Production trust acceptance is separate from this UI fixture.

Some earlier checkpoint jobs failed at a later independent gate; only their passing
steps are counted above. The final feed/activation/helper run
[41136419908](https://github.com/1bye/rake/actions/runs/41136419908) passed completely.
Its fixture uses runtime Appx parameter metadata and native package activation,
with exact C# interop compiled offline before the native check. The disposable
HTTPS server supplies real byte ranges required by App Installer.

Native release CI transports tar archives to preserve executable modes and relative
framework links. A real Developer ID-signed framework fixture passed a complete
transport roundtrip and post-extraction strict signature verification outside the
filesystem sandbox. No production private key was read or exported for this check.

## Local verification

Focused distribution/core/build/resources/platform suites and external consumers
pass. The coordinator has 24 unit/protocol tests, four extension contracts, two
metadata consumers and real worker-descendant cancellation coverage. Sixteen core
planning and two desktop metadata consumers pass. The updater suite has ten tests,
including a killed staging process and recovery of only owned partial files.
Guarded quit tests and actual native AppKit deferral supplement provider tests.

Legacy bundle tests (seven, including real Web serving/bindings) and supervisor
checks (eleven) pass. Web uses a task-local wasm-bindgen exactly matching 0.2.129;
no unrelated tool installation was replaced. The facade explicitly compiles for
`wasm32-unknown-unknown` without desktop defaults. Its six existing router/web
unused-code warnings remain; a native check is not used as Wasm evidence.

Focused strict Clippy and rustdoc pass for the new public crates and selected
updater features. Formatting, Python fixture compilation and diff whitespace
checks pass. A real verified-TLS fixture checks HTTP HEAD/MIME, exact/open/suffix
byte ranges, framing and 416 errors before App Installer retries. The entire
pre-existing workspace is not claimed clean: unrelated strict protocol lints
were observed in inherited CI. Verification is scoped to this change.

## Cost and scope accounting

The broad engine comparison was not repeated for follow-ups. Later runs select
one platform and outstanding gates; cached GUI artifacts avoid rebuilding the
nine-minute Flatpak application for compositor/repository diagnostics. Multiple
focused retries were necessary, including fixture errors; they are not described
as one successful matrix. Commits carry `[skip ci]` to avoid inherited push runs.
Two initial inherited push checks (41057985075 and 41058896200) failed on baseline
protocol lints. Later native workflows are manual.

Cancelled runs are not omitted: 41113849969 was requested too early and cancelled
after a local compile error; 41121908205 continued a real SDK build for roughly
eight minutes despite its cancel acknowledgement before replacement concurrency
stopped it. 41122853171 was cancelled with no usable native evidence. Cancellation
acknowledgement is never counted as terminal state or zero runner cost.

Validated initial profiles: macOS arm64; hosted Windows x64 MSVC; Linux x64 GNU
on Ubuntu 24.04, Fedora 43, Arch and Freedesktop 25.08. Additional CPU/libc/GPU
profiles and universal updater combinations remain separate native expansions.
Universal pure-Rust Mac slice/execution acceptance does not establish every SDK.
The media fixture establishes its real selected local inputs, not every codec SDK.

Native signing uses enrolled local profiles. CI uses only disposable certificates
and update keys. Production keys were never exported. No public release, tag,
store submission, default-branch merge or production deployment was performed.
HTTPS publication requires the documented transactional service; directory export
works with a suitable static HTTPS host. Key rotation uses a new channel and a
normally signed installer enrollment, rather than silently changing trust roots.
