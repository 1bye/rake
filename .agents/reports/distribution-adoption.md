# Distribution engine adoption

Decision: use the private tauri-bundler 2.10.4 adapter. Cargo-packager 0.11.8
remains only in the isolated comparison workspace. Its overlapping production
adapter is not shipped. Rust 1.95, edition 2024 and resolver 3 are unchanged.

The source comparison is in [the implementation plan](../plans/distribution.md#2-compare-cargo-packager-and-tauri-bundler).
Tauri covers common app, NSIS/MSI and AppImage assembly. Rake owns staged
resources, component policy, native trust, DEB/RPM/Arch/Flatpak, MSIX, Velopack,
provider integration and publication. No upstream settings or errors are public
Rake contracts. WebView installation and Tauri binary marker patching are disabled.

| Probe | Native evidence |
| --- | --- |
| macOS arm64, both engines | App contents, resource/helper execution after relocation, custom plist, linked framework, relative framework symlinks, unchanged Cargo binaries, strict external ad-hoc verification |
| Ubuntu x86_64, both engines | DEB creation/extraction, executable/resource/helper operation away from the build directory |
| Windows x86_64 MSVC, both engines | Native NSIS generation after the comparison adapter's path normalization fix |
| Selected engine production trust | Local Developer ID signing, Apple notarization, staple/ticket verification of a linked-framework fixture; no credential export |
| Selected engine actual Rake apps | Native macOS foundation and minimal example frames/resources; installed Windows minimal `rake::view!` frame/resource; pinned-SDK Flatpak minimal frames/resources on X11 and Wayland |
| Worker ownership | Real root/descendant termination, output locking, confinement, cancellation, rollback, unowned-output refusal and immutable input snapshots |
| Provider feasibility | Actual signed Sparkle Save/Cancel/relaunch, Velopack HTTPS SDK relaunch and AppImage HTTPS atomic replacement/relaunch with preserved data |

Initial comparison runs:
- https://github.com/1bye/rake/actions/runs/41057984993 (macOS/Linux passed; Windows path mismatch exposed)
- https://github.com/1bye/rake/actions/runs/41058997245 (Windows comparison passed)

Later selected-engine native evidence:
- https://github.com/1bye/rake/actions/runs/41104892784 (signed MSIX install/upgrade, actual GPUI packaging)
- https://github.com/1bye/rake/actions/runs/41108529192 (Velopack installer lifecycle)
- https://github.com/1bye/rake/actions/runs/41111628107 (APT/RPM/pacman repositories and real AppImage SDK update)
- https://github.com/1bye/rake/actions/runs/41130631879 (Velopack SDK update, native DLLs, localized MSIX and NSIS registration passed; MSI/UI follow-up required)
- https://github.com/1bye/rake/actions/runs/41132236560 (MSI lifecycle/registrations and installed minimal Windows UI passed; App Installer test CA setup needed correction)
- https://github.com/1bye/rake/actions/runs/41130639063 (AppImage failure scenarios, Linux legacy compatibility and pinned-SDK Flatpak X11 frame passed; OSTree repository initialization needed correction)

Dependency measurement on macOS arm64, locked normal dependency graphs:
`rake-cli` has 411 unique package name/version pairs and includes only the selected
Tauri engine; the minimal GUI example has 401 and includes neither packaging
engine nor the coordinator/platform packaging crates. Shared transitive packages
mean these totals are not an incremental cost comparison. The local unoptimized
CLI built with debug information disabled measured 75,061,840 bytes. Runtime
applications depend on portable core/resource contracts; packagers stay in the
CLI graph. These are concrete graph/build measurements, not a cold-build benchmark.

Native tools are explicit: doctor reports prerequisites and native execution
validates them. Upstream-managed installer tools use the pinned engine's verified
cache. Component offline mode covers verified component inputs, not every native
installer tool download. Flatpak records exact SDK/runtime commits and verified
Rust component hashes. Missing native tools/credentials are errors, not skipped
release gates. Production local signing references never contain private key data.

The complete Flatpak native handoff/repository and X11/Wayland acceptance passed
in [41133766124](https://github.com/1bye/rake/actions/runs/41133766124). Detailed
phase acceptance is tracked in [implementation progress](distribution-progress.md). Additional CPU/libc/desktop
profiles require their own native tests; this report does not establish all Linux
distributions or universal native updater combinations.
