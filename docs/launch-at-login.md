# Launch at login

Enable `rake = { ..., features = ["launch-at-login"] }`.
This feature works with `default-features = false`. The underlying
`rake-autostart` crate follows the GPUI-independent tray crate boundary:
it owns native settings, while applications own preferences and UI. It does not
change tray APIs, app startup policy, or the minimum deployment version.

```rust,no_run
use rake::login_item::{LaunchAtLogin, Status};

async fn set_login(enabled: bool) -> Result<Status, rake::login_item::Error> {
    // Match rake-dist's identifier and CFBundleIdentifier, not a display name.
    let login = LaunchAtLogin::new("dev.example.app")?;
    if enabled { login.enable().await } else { login.disable().await }
}
```

Construction and drop never change OS settings. Explicit operations are idempotent;
read the returned status rather than saving an optimistic checkbox value.
`status()` queries the OS again, so changes made in system settings are visible.
`RequiresApproval` means the application cannot bypass user consent.
All methods return `Result<Status, Error>`; an unsupported environment includes a reason.
Filesystem/registry calls are synchronous within the async API; portal requests
await user interaction. Applications should avoid repeated calls during rendering.

Optional `.args(["--background"])` sets login-only arguments on Windows/Linux.
The macOS main-app service cannot specify arguments. Keep a stable executable
location on Windows/Linux; enabling again updates a moved application's path.

| Platform / package | Mechanism and limitations |
| --- | --- |
| macOS 13+ signed .app | SMAppService.mainAppService; bundle ID must match the supplied ID. Enabled, not registered, requires approval and not found are mapped explicitly. Invalid signatures return a native service error. |
| macOS <13 | Unsupported at runtime, even if rake-dist's macos.minimum_system_version permits the app to run. The distribution minimum is preserved; set it to 13.0 if the entire app requires this feature. |
| macOS cargo run / loose executable | Unsupported with a signed-bundle explanation; no launchd file or legacy login-item fallback. |
| Windows unpackaged / Velopack | Per-user HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run, value named exactly after the app ID. The current executable and optional arguments use Windows command-line quoting. Commands longer than 260 UTF-16 units are rejected. |
| Windows Startup Apps disabled | Known disabled StartupApproved\\Run states (3/7) return RequiresApproval. Enable does not clear the user's system preference; re-enable in Windows Settings / Task Manager. This undocumented binary format is best effort, not an assurance against all Windows startup policies. |
| Windows MSIX / package identity | Unsupported: the Run backend does not bypass registry virtualization. These packages need manifest StartupTask integration, which this API does not implement. |
| Linux desktop | Atomic replacement of $XDG_CONFIG_HOME/autostart/<app-id>.desktop, default $HOME/.config. Relative XDG_CONFIG_HOME is ignored per XDG conventions. Hidden=true reports Disabled. Status describes registration, not a guarantee that a particular desktop will launch it. |
| Linux AppImage | $APPIMAGE replaces the current executable (which may be a temporary mount's AppRun). Both paths must be absolute. Desktop string/Exec escaping protects spaces, quotes, backslashes, dollar signs, backticks and literal percent signs; invalid line breaks or executable = are rejected. |
| Flatpak | Detects /.flatpak-info; uses org.freedesktop.portal.Background.RequestBackground through the existing zbus/async-io dependency family, never a sandbox-local autostart file. May prompt. The enable/disable response reports granted autostart. Cancellation/denial returns RequiresApproval. |
| Flatpak status | The Background portal has no read-only autostart query. status() returns Unsupported explaining this; retain the most recent request result for the current UI session. With no custom args, the portal uses the packaged desktop Exec. |
| Snap | Detects SNAP and returns Unsupported. Confinement and snap-specific startup integration are not bypassed by writing a desktop file. |
| Web / wasm / other platforms | Compiles and returns Unsupported. |

The tray workbench adds a **Launch at login** menu checkbox and displays the actual
result in its event history. It changes nothing until selected. Flatpak retains
the last portal response for the checkbox because it cannot query existing state.
Run a signed packaged build to use the menu on macOS. A separate read-only probe
is available as `cargo run -p rake-autostart --example status`.

## Uninstall and updates

Rake generates Windows Velopack and MSIX packages; it has no MSI or NSIS generator.

- **Velopack:** the existing before-uninstall callback removes the current user's
  Run value for the embedded distribution identifier only when the command points
  to that installation's main executable, with or without login arguments.
  Call `rake::early_startup!()` before application construction (automatic with
  `#[rake::app]`) and enable `updates-velopack`, as required for Rake's Velopack
  packages. Cleanup runs even when no protocol/file associations are configured.
  Update callbacks preserve login registration and user-disabled StartupApproved
  preferences. Custom IDs, other user accounts, redirected Run values, and
  portable copies cannot be reliably identified and are left alone.
- **MSIX:** this backend creates no entry for a packaged process. No custom
  uninstall executable is invented; package-owned state belongs to Windows.
- **Windows portable archives:** there is no uninstall step. Call disable before
  deleting the app or remove its Run value from Windows startup settings.
- **Linux deb/rpm/archives/AppImage:** package removal cannot safely enumerate and
  modify each user's home directory. Per-user autostart files remain; disable
  before removing the app, or delete the named desktop file manually. Flatpak
  startup permission is managed by the portal/Flatpak, not package scripts.
- **macOS:** login service registration belongs to the app bundle and becomes
  unavailable when the app is removed; there is no Rake launchd file to clean.
  Disable before uninstall to unregister immediately. System Settings may retain
  a stale display record until macOS refreshes it.

## Tested on

No packaged login/logout or uninstall end-to-end qualification is claimed.

| Host / target | Evidence |
| --- | --- |
| devbox-ts Linux x86_64 (headless) | Real temporary-XDG filesystem round trips, idempotence, Hidden state, command escaping, AppImage selection, inode replacement, and external consumer API tests. No real user-profile writes or desktop login cycle. |
| Linux wasm32-unknown-unknown | Core compile check; no browser runtime test. |
| macOS 26.6.2 arm64, Rust 1.98.0 | Core compile, strict Clippy, 4 unit tests (all native status mappings plus portable tests), doctest, and tray-example compile passed. Read-only unbundled probe returned Unsupported with the signed-.app explanation. Signed registration/login cycles not run. |
| Windows x86_64-pc-windows-msvc (cross-check on Mac) | Core strict Clippy with all Cargo targets passed, including registry and installed-value removal code. Portable quoting/ownership tests ran on Linux and Mac. Registry calls were compile-checked only. Broader rake-updates/Velopack cross-check blocked by ring's cross C build (missing Windows assert.h). No Windows desktop or uninstall run. |
| Linux GPUI example | Compile blocked in yeslogic-fontconfig-sys: pkg-config is absent and fontconfig development prerequisites are unavailable. No packages installed. The example compiled on Mac. |
| Flatpak / Snap | Sandbox paths implemented; no installed sandbox/portal end-to-end qualification. |

## Native references

- [Apple SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice)
- [Background portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Background.html)
- [Desktop Exec escaping](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html)
- [Windows Run keys](https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys)
- [MSIX virtualization](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes)
